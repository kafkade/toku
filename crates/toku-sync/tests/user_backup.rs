//! Per-user encrypted backup / restore tests (issue #206, ADR-014 D6).
//!
//! The admin-scoped backup exports only ciphertext (op payloads, snapshots) and
//! the opaque metadata the relay already holds; restore re-ingests it
//! idempotently under the same account. These tests assert the round-trip, the
//! idempotency, the admin gate, and — critically — that the bundle carries no
//! plaintext (zero-knowledge preserved end-to-end).

mod harness;

use harness::{SimulatedDevice, TestServer};
use serde_json::json;
use sha2::{Digest, Sha256};
use srp::ClientG2048;
use tokio::runtime::Runtime;
use toku_core::{EntityType, HybridClock, OpType};
use toku_sync::db::SyncDatabase;
use uuid::Uuid;

fn rt() -> Runtime {
    Runtime::new().expect("tokio runtime")
}

async fn post(
    base_url: &str,
    path: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    let mut req = reqwest::Client::new().post(format!("{base_url}{path}"));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.json(&body).send().await.expect("request sent");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn get(base_url: &str, path: &str, token: Option<&str>) -> (reqwest::StatusCode, String) {
    let mut req = reqwest::Client::new().get(format!("{base_url}{path}"));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.expect("request sent");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

fn hlc_for(device_id: Uuid) -> String {
    HybridClock::new(&device_id).now().to_canonical()
}

// ── SRP account helpers (mirrors user_accounts.rs) ────────────────────────────

fn salt_for(email: &str) -> Vec<u8> {
    Sha256::digest(format!("salt:{email}").as_bytes())[..16].to_vec()
}

fn verifier_hex(email: &str, password: &str, salt: &[u8]) -> String {
    let client = ClientG2048::<Sha256>::new();
    hex::encode(client.compute_verifier(email.as_bytes(), password.as_bytes(), salt))
}

/// Sign up an account and return its `user_id`.
fn signup(server: &TestServer, email: &str, password: &str) -> String {
    let salt = salt_for(email);
    let body = json!({
        "email": email,
        "srp_salt": hex::encode(&salt),
        "srp_verifier": verifier_hex(email, password, &salt),
    });
    let (status, text) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/signup",
        None,
        body,
    ));
    assert_eq!(status, reqwest::StatusCode::OK, "signup failed: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("signup json");
    v["user_id"].as_str().expect("user_id").to_string()
}

/// Full SRP login; returns the user-session token.
fn login(server: &TestServer, email: &str, password: &str) -> String {
    use rand::RngExt;
    let client = ClientG2048::<Sha256>::new();
    let mut a = [0u8; 48];
    rand::rng().fill(&mut a);
    let a_pub = client.compute_public_ephemeral(&a);

    let (_, challenge) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/challenge",
        None,
        json!({ "email": email, "client_public_a": hex::encode(&a_pub) }),
    ));
    let challenge: serde_json::Value = serde_json::from_str(&challenge).expect("challenge json");
    let challenge_id = challenge["challenge_id"].as_str().expect("challenge_id");
    let server_b = hex::decode(challenge["server_public_b"].as_str().unwrap()).unwrap();
    let salt = hex::decode(challenge["srp_salt"].as_str().unwrap()).unwrap();
    let verifier = client
        .process_reply(&a, email.as_bytes(), password.as_bytes(), &salt, &server_b)
        .expect("process_reply");
    let m1 = hex::encode(verifier.proof());

    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/verify",
        None,
        json!({ "challenge_id": challenge_id, "client_proof_m1": m1 }),
    ));
    assert_eq!(status, reqwest::StatusCode::OK, "login failed: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("verify json");
    v["session_token"]
        .as_str()
        .expect("session_token")
        .to_string()
}

/// Attach `library_id` to `user_id` on the server (models admin adoption).
fn attach_library(server: &TestServer, library_id: &str, user_id: &str) {
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .execute(
            "UPDATE libraries SET user_id = ?1 WHERE id = ?2",
            rusqlite::params![user_id, library_id],
        )
        .expect("attach library");
}

/// Push one encrypted op into `library_id` via `device`, returning the secret
/// title used (so a test can assert it never appears in the backup bundle).
fn push_encrypted_op(server: &TestServer, device: &SimulatedDevice, secret_title: &str) {
    let key = device.sync_key(server);
    let entity_id = Uuid::now_v7();
    let envelope = toku_core::encrypt_fields(
        &key,
        &json!({ "title": secret_title }),
        &EntityType::Book,
        &entity_id,
        &OpType::Create,
    )
    .expect("encrypt fields");
    let op = json!({
        "op_id": Uuid::now_v7().to_string(),
        "device_id": device.device_id().to_string(),
        "hlc": hlc_for(device.device_id()),
        "entity_type": EntityType::Book.as_str(),
        "entity_id": entity_id.to_string(),
        "op_type": OpType::Create.as_str(),
        "payload": serde_json::to_value(&envelope).unwrap(),
    });
    let token = device.auth_token(server);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        Some(&token),
        json!({ "ops": [op] }),
    ));
    assert!(status.is_success(), "seed push failed: {body}");
}

#[test]
fn backup_restore_round_trips_and_is_idempotent() {
    let server = TestServer::start();
    // First signup bootstraps the admin.
    let admin_id = signup(&server, "admin@example.com", "correct horse");
    let admin_token = login(&server, "admin@example.com", "correct horse");

    let device = SimulatedDevice::register(&server, "backup-lib", "device-a", None);
    attach_library(&server, "backup-lib", &admin_id);
    let secret = "A Very Secret Book Title";
    push_encrypted_op(&server, &device, secret);

    // Export the backup.
    let (status, body) = rt().block_on(get(
        server.base_url(),
        &format!("/api/v1/admin/users/{admin_id}/backup"),
        Some(&admin_token),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "backup export failed: {body}"
    );
    let bundle: serde_json::Value = serde_json::from_str(&body).expect("bundle json");

    assert_eq!(bundle["user_id"], admin_id);
    assert_eq!(bundle["ops"].as_array().unwrap().len(), 1);
    assert_eq!(bundle["libraries"].as_array().unwrap().len(), 1);

    // Zero-knowledge: the plaintext title must appear nowhere in the bundle.
    assert!(
        !body.contains(secret),
        "backup bundle must not contain plaintext content"
    );

    // Wipe the ops and restore from the bundle.
    {
        let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
        db.conn.execute("DELETE FROM ops", []).expect("clear ops");
    }
    let (status, body) = rt().block_on(post(
        server.base_url(),
        &format!("/api/v1/admin/users/{admin_id}/backup/restore"),
        Some(&admin_token),
        bundle.clone(),
    ));
    assert_eq!(status, reqwest::StatusCode::OK, "restore failed: {body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["ops_restored"], 1);

    let op_count: i64 = {
        let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
        db.conn
            .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(op_count, 1, "restore must re-ingest the op");

    // Idempotent: restoring again inserts nothing new.
    let (status, body) = rt().block_on(post(
        server.base_url(),
        &format!("/api/v1/admin/users/{admin_id}/backup/restore"),
        Some(&admin_token),
        bundle,
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "second restore failed: {body}"
    );
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["ops_restored"], 0, "re-restore must be a no-op");
}

#[test]
fn backup_requires_admin() {
    let server = TestServer::start();
    // Admin bootstrap, then a plain user (registration opened by admin first).
    let _admin_id = signup(&server, "admin@example.com", "correct horse");
    let admin_token = login(&server, "admin@example.com", "correct horse");
    let (status, _) = rt().block_on(async {
        let resp = reqwest::Client::new()
            .put(format!("{}/api/v1/admin/registration", server.base_url()))
            .bearer_auth(&admin_token)
            .json(&json!({ "open": true }))
            .send()
            .await
            .expect("open registration");
        (resp.status(), resp.text().await.unwrap_or_default())
    });
    assert_eq!(status, reqwest::StatusCode::OK);

    let user_id = signup(&server, "user@example.com", "hunter2 hunter2");
    let user_token = login(&server, "user@example.com", "hunter2 hunter2");

    // A plain user cannot back up any account (admin-only capability).
    let (status, _) = rt().block_on(get(
        server.base_url(),
        &format!("/api/v1/admin/users/{user_id}/backup"),
        Some(&user_token),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::FORBIDDEN,
        "non-admin backup must be forbidden"
    );
}

#[test]
fn restore_rejects_mismatched_user_id() {
    let server = TestServer::start();
    let admin_id = signup(&server, "admin@example.com", "correct horse");
    let admin_token = login(&server, "admin@example.com", "correct horse");

    let bundle = json!({
        "version": 1,
        "user_id": "some-other-user",
        "email": "x@example.com",
        "exported_at": "2024-01-01 00:00:00",
        "libraries": [],
        "ops": [],
        "snapshots": [],
    });
    let (status, _) = rt().block_on(post(
        server.base_url(),
        &format!("/api/v1/admin/users/{admin_id}/backup/restore"),
        Some(&admin_token),
        bundle,
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "restore must reject a bundle whose user_id != path id"
    );
}
