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
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::db::SyncDatabase;

fn build_router(db_path: PathBuf) -> Router {
    // Routes that require authentication
    let authenticated = Router::new()
        .route("/api/v1/devices", get(handlers::list_devices))
        .route("/api/v1/devices/{id}", delete(handlers::delete_device))
        .route("/api/v1/push", post(handlers::push_ops))
        .route("/api/v1/pull", get(handlers::pull_ops))
        .route("/api/v1/pull/all", get(handlers::pull_all_ops))
        .route("/api/v1/salt", get(handlers::get_salt))
        .route("/api/v1/snapshot", get(handlers::download_snapshot))
        .route("/api/v1/snapshot", post(handlers::upload_snapshot))
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
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50 MB (rekey may be large)
        .layer(TraceLayer::new_for_http())
        .with_state(db_path)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Lightweight health-check mode for container orchestration. Runs before the
    // full config is parsed so it works on shell-less images (distroless/scratch):
    //   `toku-sync healthcheck` exits 0 if GET /health returns 200, else 1.
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(run_healthcheck());
    }

    let config = Config::parse();
    init_tracing(&config.log_level);
    let db_path = config.db_path();

    // Run migrations once at startup
    SyncDatabase::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to initialize database: {e}"))?;

    let app = build_router(db_path);
    let addr = config.bind_addr();

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("toku-sync listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Initialise the tracing subscriber. `RUST_LOG` takes precedence; otherwise the
/// configured log level (from `--log-level` / `TOKU_SYNC_LOG_LEVEL`) is used.
fn init_tracing(log_level: &str) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Perform a minimal HTTP `GET /health` against the local server using only the
/// standard library (no extra runtime deps), so it runs on a `scratch` image.
/// Returns a process exit code: 0 = healthy, 1 = unhealthy.
fn run_healthcheck() -> i32 {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let port = std::env::var("TOKU_SYNC_PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("127.0.0.1:{port}");

    let result = (|| -> std::io::Result<bool> {
        let mut stream = TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream
            .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"))
    })();

    match result {
        Ok(true) => 0,
        Ok(false) => {
            eprintln!("healthcheck: server at {addr} did not return 200");
            1
        }
        Err(e) => {
            eprintln!("healthcheck: failed to reach {addr}: {e}");
            1
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    tracing::info!("shutting down");
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
        assert!(
            push["new_cursor"].is_string(),
            "push should return new_cursor"
        );
        assert_eq!(push["new_cursor"], "dup-1");

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
    async fn snapshot_download_returns_404_when_none_exists() {
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

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn snapshot_upload_and_download_round_trip() {
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
                            "library_id": "lib-snap",
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

        // Push some ops first
        app.clone()
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
                                    "op_id": "snap-op-1",
                                    "device_id": "d1",
                                    "hlc": "2026-01-01T00:00:00.000Z-0001-aaaaaaaaaaaa",
                                    "entity_type": "book",
                                    "entity_id": "b1",
                                    "op_type": "create",
                                    "payload": {"title": "Dune"}
                                },
                                {
                                    "op_id": "snap-op-2",
                                    "device_id": "d1",
                                    "hlc": "2026-01-01T00:00:01.000Z-0001-aaaaaaaaaaaa",
                                    "entity_type": "book",
                                    "entity_id": "b1",
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

        // Upload a snapshot at a HLC that covers the first op
        let snapshot_hlc = "2026-01-01T00:00:00.500Z-0000-aaaaaaaaaaaa";
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/snapshot")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "snapshot_json": "{\"version\":1,\"books\":[{\"title\":\"Dune\"}]}",
                            "hlc_at_snapshot": snapshot_hlc,
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
        let upload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // First op (hlc ...00.000Z) should be pruned, second (...01.000Z) kept
        assert_eq!(upload["ops_pruned"], 1);

        // Download the snapshot
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/snapshot")
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
        let download: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(download["hlc_at_snapshot"], snapshot_hlc);
        assert!(download["snapshot_json"].as_str().unwrap().contains("Dune"));

        // Verify the first op was pruned — pull/all should only return the second
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull/all")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ops = pull["ops"].as_array().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["op_id"], "snap-op-2");

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

    #[tokio::test]
    async fn pull_excludes_own_device_ops() {
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
                            "device_name": "dev-a"
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
        let device_a_id = reg_a["device_id"].as_str().unwrap().to_string();

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
                            "device_name": "dev-b"
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

        // Device A pushes an op
        let ops_json = serde_json::json!({
            "ops": [{
                "op_id": "op-from-a",
                "device_id": &device_a_id,
                "hlc": "2026-01-01T00:00:00.000Z-0001-aaaaaaaaaaaa",
                "entity_type": "book",
                "entity_id": "b1",
                "op_type": "create",
                "payload": {"title": "Test Book"}
            }]
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token_a}"))
                    .body(Body::from(serde_json::to_string(&ops_json).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Device A pulls — should NOT see its own op
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull")
                    .header("Authorization", format!("Bearer {token_a}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pull["ops"].as_array().unwrap().len(), 0);
        assert_eq!(pull["has_more"], false);

        // Device B pulls — SHOULD see device A's op
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull")
                    .header("Authorization", format!("Bearer {token_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pull["ops"].as_array().unwrap().len(), 1);
        assert_eq!(pull["ops"][0]["op_id"], "op-from-a");
        assert_eq!(pull["has_more"], false);
        assert!(pull["cursor"].is_string());

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn push_returns_new_cursor() {
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
        let reg: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = reg["auth_token"].as_str().unwrap().to_string();

        // Push multiple ops
        let ops_json = serde_json::json!({
            "ops": [
                {
                    "op_id": "op-1",
                    "device_id": "d",
                    "hlc": "2026-01-01T00:00:00.000Z-0001-d",
                    "entity_type": "book",
                    "entity_id": "b1",
                    "op_type": "create",
                    "payload": {}
                },
                {
                    "op_id": "op-2",
                    "device_id": "d",
                    "hlc": "2026-01-01T00:00:01.000Z-0001-d",
                    "entity_type": "book",
                    "entity_id": "b1",
                    "op_type": "update",
                    "payload": {"title": "Updated"}
                }
            ]
        });

        let resp = app
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
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let push: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(push["accepted"], 2);
        assert_eq!(push["new_cursor"], "op-2");

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn pull_has_more_pagination() {
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
                            "device_name": "pusher"
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
        let reg_push: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let push_token = reg_push["auth_token"].as_str().unwrap().to_string();

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
                            "device_name": "puller"
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
        let reg_pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let pull_token = reg_pull["auth_token"].as_str().unwrap().to_string();

        // Push 5 ops from the pusher device (small batch to verify has_more=false)
        let ops: Vec<serde_json::Value> = (0..5)
            .map(|i| {
                serde_json::json!({
                    "op_id": format!("op-{i}"),
                    "device_id": "d",
                    "hlc": format!("2026-01-01T00:00:{i:02}.000Z-0001-d"),
                    "entity_type": "book",
                    "entity_id": format!("b{i}"),
                    "op_type": "create",
                    "payload": {}
                })
            })
            .collect();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {push_token}"))
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({ "ops": ops })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Puller pulls — should get all 5 and has_more=false
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull")
                    .header("Authorization", format!("Bearer {pull_token}"))
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
        assert_eq!(pull["ops"].as_array().unwrap().len(), 5);
        assert_eq!(pull["has_more"], false);
        assert!(pull["cursor"].is_string());

        // Pull again with cursor — should get empty
        let cursor = pull["cursor"].as_str().unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/pull?since={cursor}"))
                    .header("Authorization", format!("Bearer {pull_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pull: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pull["ops"].as_array().unwrap().len(), 0);
        assert_eq!(pull["has_more"], false);

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn rekey_replaces_ops_and_updates_salt() {
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
                            "library_id": "lib-rekey",
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

        // Push two ops
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
                                    "op_id": "rk-op-1",
                                    "device_id": "d1",
                                    "hlc": "2026-01-01T00:00:00.000Z-0001-d1",
                                    "entity_type": "book",
                                    "entity_id": "b1",
                                    "op_type": "create",
                                    "payload": {"title": "Dune"}
                                },
                                {
                                    "op_id": "rk-op-2",
                                    "device_id": "d1",
                                    "hlc": "2026-01-01T00:00:01.000Z-0001-d1",
                                    "entity_type": "book",
                                    "entity_id": "b1",
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

        // Rekey with "re-encrypted" ops (different payloads simulating re-encryption)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rekey")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "new_salt": "bmV3LXNhbHQ=",
                            "ops": [
                                {
                                    "op_id": "rk-op-1",
                                    "device_id": "d1",
                                    "hlc": "2026-01-01T00:00:00.000Z-0001-d1",
                                    "entity_type": "book",
                                    "entity_id": "b1",
                                    "op_type": "create",
                                    "payload": {"ev": 1, "alg": "aes-256-gcm", "ciphertext": "new-ct-1"}
                                },
                                {
                                    "op_id": "rk-op-2",
                                    "device_id": "d1",
                                    "hlc": "2026-01-01T00:00:01.000Z-0001-d1",
                                    "entity_type": "book",
                                    "entity_id": "b1",
                                    "op_type": "update",
                                    "payload": {"ev": 1, "alg": "aes-256-gcm", "ciphertext": "new-ct-2"}
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
        let rekey: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rekey["ops_replaced"], 2);
        assert_eq!(rekey["new_salt"], "bmV3LXNhbHQ=");

        // Verify salt was updated
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/salt")
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
        let salt: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(salt["salt"], "bmV3LXNhbHQ=");

        // Verify ops are the re-encrypted versions via pull/all
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pull/all")
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
        let ops = pull["ops"].as_array().unwrap();
        assert_eq!(ops.len(), 2);
        // Verify payloads are the re-encrypted versions
        assert_eq!(ops[0]["payload"]["ciphertext"], "new-ct-1");
        assert_eq!(ops[1]["payload"]["ciphertext"], "new-ct-2");

        cleanup(&db_path);
    }

    #[tokio::test]
    async fn push_blocked_during_rekey() {
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
                            "library_id": "lib-lock",
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

        // Manually set rekey lock
        {
            let db = SyncDatabase::open_no_migrate(&db_path).unwrap();
            db.conn
                .execute(
                    "UPDATE libraries SET rekey_in_progress = 1 WHERE id = 'lib-lock'",
                    [],
                )
                .unwrap();
        }

        // Push should be rejected
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
                            "ops": [{
                                "op_id": "blocked-op",
                                "device_id": "d1",
                                "hlc": "2026-01-01T00:00:00.000Z-0001-d1",
                                "entity_type": "book",
                                "entity_id": "b1",
                                "op_type": "create",
                                "payload": {}
                            }]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err["error"].as_str().unwrap().contains("re-keyed"));

        cleanup(&db_path);
    }
}
