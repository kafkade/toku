mod auth;
mod config;
mod db;
mod error;
mod handlers;
mod models;

use std::path::PathBuf;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post};
use clap::Parser;

use crate::config::Config;
use crate::db::SyncDatabase;

fn build_router(db_path: PathBuf) -> Router {
    // Routes that require authentication
    let authenticated = Router::new()
        .route("/api/v1/devices", get(handlers::list_devices))
        .route("/api/v1/devices/{id}", delete(handlers::delete_device))
        .route("/api/v1/push", post(handlers::push_ops))
        .route("/api/v1/pull", get(handlers::pull_ops))
        .route("/api/v1/snapshot", get(handlers::snapshot))
        .route("/api/v1/rekey", post(handlers::rekey))
        .layer(middleware::from_fn_with_state(
            db_path.clone(),
            auth::require_auth,
        ));

    // Public routes
    Router::new()
        .route("/health", get(handlers::health))
        .route("/api/v1/register", post(handlers::register))
        .merge(authenticated)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // 2 MB
        .with_state(db_path)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    let db_path = config.db_path();

    // Run migrations once at startup
    SyncDatabase::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to initialize database: {e}"))?;

    let app = build_router(db_path);
    let addr = config.bind_addr();

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("toku-sync listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    eprintln!("\nshutting down...");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_db_path() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("toku-sync-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        SyncDatabase::open(&path).unwrap();
        path
    }

    fn cleanup(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[tokio::test]
    async fn health_check() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn register_and_list_devices() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register a device
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "test-laptop"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let register: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(register["library_id"], "lib-1");
        assert!(!register["auth_token"].as_str().unwrap().is_empty());
        let token = register["auth_token"].as_str().unwrap().to_string();

        // List devices
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/devices")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let devices: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["device_name"], "test-laptop");

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn unauthorized_without_token() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/devices")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn push_and_pull_ops() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "test-device"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let register: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = register["auth_token"].as_str().unwrap().to_string();

        // Push ops
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "ops": [
                                {
                                    "op_id": "op-1",
                                    "device_id": "dev-1",
                                    "hlc": "2026-06-01T00:00:00.000Z-0001-dev1",
                                    "entity_type": "book",
                                    "entity_id": "book-1",
                                    "op_type": "create",
                                    "payload": {"title": "Dune"}
                                },
                                {
                                    "op_id": "op-2",
                                    "device_id": "dev-1",
                                    "hlc": "2026-06-01T00:00:01.000Z-0001-dev1",
                                    "entity_type": "book",
                                    "entity_id": "book-1",
                                    "op_type": "update",
                                    "payload": {"rating": 9}
                                }
                            ]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let push: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(push["accepted"], 2);
        assert_eq!(push["duplicates"], 0);

        // Pull all ops
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pull["ops"].as_array().unwrap().len(), 2);
        assert_eq!(pull["cursor"], "op-2");

        // Pull since cursor (should return empty)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull?since=op-2")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(pull["ops"].as_array().unwrap().is_empty());

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn push_duplicate_ops_are_idempotent() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "dev"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let register: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = register["auth_token"].as_str().unwrap().to_string();

        let ops_json = serde_json::json!({
            "ops": [{
                "op_id": "dup-1",
                "device_id": "d",
                "hlc": "2026-01-01T00:00:00.000Z-0001-d",
                "entity_type": "book",
                "entity_id": "b1",
                "op_type": "create",
                "payload": {}
            }]
        });

        // Push once
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&ops_json).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let push: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(push["accepted"], 1);

        // Push same op again
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&ops_json).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let push: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(push["accepted"], 0);
        assert_eq!(push["duplicates"], 1);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn delete_device() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register two devices
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "device-a"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let reg_a: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token_a = reg_a["auth_token"].as_str().unwrap().to_string();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "device-b"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let reg_b: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let device_b_id = reg_b["device_id"].as_str().unwrap().to_string();

        // Delete device B using device A's token
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/devices/{device_b_id}"))
                    .header("Authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify only one device remains
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/devices")
                    .header("Authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let devices: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["device_name"], "device-a");

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn register_validates_required_fields() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "",
                            "device_name": "test"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn snapshot_returns_501() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register to get a token
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "dev"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let register: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = register["auth_token"].as_str().unwrap().to_string();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/snapshot")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn push_rejects_oversized_batch() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "dev"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let register: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = register["auth_token"].as_str().unwrap().to_string();

        // Build a batch with 1001 ops
        let ops: Vec<serde_json::Value> = (0..1001)
            .map(|i| {
                serde_json::json!({
                    "op_id": format!("op-{i}"),
                    "device_id": "d",
                    "hlc": format!("2026-01-01T00:00:{i:02}.000Z-0001-d"),
                    "entity_type": "book",
                    "entity_id": "b1",
                    "op_type": "create",
                    "payload": {}
                })
            })
            .collect();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({ "ops": ops })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn register_returns_base64url_token() {
        use base64::Engine;

        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "test"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = json["auth_token"].as_str().unwrap();

        // Must be valid base64url with no padding
        assert!(!token.contains('+'), "token must not contain '+'");
        assert!(!token.contains('/'), "token must not contain '/'");
        assert!(!token.contains('='), "token must not contain '='");

        // Must decode to exactly 32 bytes (256 bits)
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token)
            .expect("token must be valid base64url");
        assert_eq!(decoded.len(), 32, "token must be 256 bits");

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn cannot_delete_own_device() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register a device
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "my-device"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let reg: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = reg["auth_token"].as_str().unwrap().to_string();
        let device_id = reg["device_id"].as_str().unwrap().to_string();

        // Try to delete self — should be 403 Forbidden
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/devices/{device_id}"))
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // Verify the device still exists
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/devices")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let devices: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(devices.len(), 1);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn deleted_device_token_is_invalidated() {
        let db_path = test_db_path();
        let app = build_router(db_path.clone());

        // Register two devices
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "keeper"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let reg_a: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token_a = reg_a["auth_token"].as_str().unwrap().to_string();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "library_id": "lib-1",
                            "device_name": "doomed"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let reg_b: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token_b = reg_b["auth_token"].as_str().unwrap().to_string();
        let device_b_id = reg_b["device_id"].as_str().unwrap().to_string();

        // Delete device B using device A's token
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/devices/{device_b_id}"))
                    .header("Authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Device B's token should now be rejected
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/devices")
                    .header("Authorization", format!("Bearer {token_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        cleanup(&db_path);
    }
}
