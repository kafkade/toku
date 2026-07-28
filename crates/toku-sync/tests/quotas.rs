//! Per-user storage / op-count quota tests (issue #206, ADR-014 D2).
//!
//! The quota check enforces per-account ceilings on the push and snapshot
//! ingest paths while preserving zero-knowledge — it reads only ciphertext
//! *sizes* and *counts*, never plaintext. These tests assert:
//!   * an over-byte push is rejected with 413 and nothing is persisted;
//!   * an over-op-count push is rejected with 413;
//!   * the instance-wide default ceiling applies to owned accounts;
//!   * a library with no owning account (self-hosted open relay) is exempt.

mod harness;

use harness::{SimulatedDevice, TestServer};
use serde_json::json;
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
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    let resp = reqwest::Client::new()
        .post(format!("{base_url}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("request sent");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

fn hlc_for(device_id: Uuid) -> String {
    HybridClock::new(&device_id).now().to_canonical()
}

/// An encrypted `Create` op for `device`, targeting a fresh entity.
fn encrypted_op(device: &SimulatedDevice, server: &TestServer) -> serde_json::Value {
    let key = device.sync_key(server);
    let entity_id = Uuid::now_v7();
    let envelope = toku_core::encrypt_fields(
        &key,
        &json!({ "title": "quota probe payload that is comfortably over ten bytes" }),
        &EntityType::Book,
        &entity_id,
        &OpType::Create,
    )
    .expect("encrypt fields");
    json!({
        "op_id": Uuid::now_v7().to_string(),
        "device_id": device.device_id().to_string(),
        "hlc": hlc_for(device.device_id()),
        "entity_type": EntityType::Book.as_str(),
        "entity_id": entity_id.to_string(),
        "op_type": OpType::Create.as_str(),
        "payload": serde_json::to_value(&envelope).unwrap(),
    })
}

/// Attach `library_id` to a freshly-created owning account and return that
/// account's id. Mirrors what admin adoption / account enrollment does, but
/// keeps the test focused on quota logic.
fn own_library(server: &TestServer, library_id: &str) -> String {
    let user_id = Uuid::now_v7().to_string();
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .execute(
            "INSERT INTO users (id, email, srp_salt, srp_verifier, created_at)
             VALUES (?1, ?2, 'aa', 'bb', datetime('now'))",
            rusqlite::params![user_id, format!("{user_id}@example.com")],
        )
        .expect("insert owning user");
    db.conn
        .execute(
            "UPDATE libraries SET user_id = ?1 WHERE id = ?2",
            rusqlite::params![user_id, library_id],
        )
        .expect("attach library to user");
    user_id
}

fn set_user_quota(
    server: &TestServer,
    user_id: &str,
    max_bytes: Option<i64>,
    max_ops: Option<i64>,
) {
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .execute(
            "INSERT INTO user_quota (user_id, max_bytes, max_ops, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            rusqlite::params![user_id, max_bytes, max_ops],
        )
        .expect("set user quota");
}

fn stored_op_count(server: &TestServer, library_id: &str) -> i64 {
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .query_row(
            "SELECT COUNT(*) FROM ops WHERE library_id = ?1",
            [library_id],
            |r| r.get(0),
        )
        .expect("count ops")
}

#[test]
fn over_byte_quota_rejected_with_413() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "quota-bytes", "device-a", None);
    let token = device.auth_token(&server);
    let user_id = own_library(&server, "quota-bytes");
    // A 10-byte ceiling is smaller than any real encrypted envelope.
    set_user_quota(&server, &user_id, Some(10), None);

    let op = encrypted_op(&device, &server);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        &token,
        json!({ "ops": [op] }),
    ));

    assert_eq!(
        status,
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "over-byte push must be rejected with 413; body: {body}"
    );
    assert_eq!(
        stored_op_count(&server, "quota-bytes"),
        0,
        "a quota-rejected op must not be persisted"
    );
}

#[test]
fn over_op_count_quota_rejected_with_413() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "quota-ops", "device-a", None);
    let token = device.auth_token(&server);
    let user_id = own_library(&server, "quota-ops");
    // Generous byte ceiling, zero op-count headroom.
    set_user_quota(&server, &user_id, Some(1_000_000), Some(0));

    let op = encrypted_op(&device, &server);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        &token,
        json!({ "ops": [op] }),
    ));

    assert_eq!(
        status,
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "over-op-count push must be rejected with 413; body: {body}"
    );
    assert_eq!(stored_op_count(&server, "quota-ops"), 0);
}

#[test]
fn instance_default_ceiling_applies_to_owned_accounts() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "quota-default", "device-a", None);
    let token = device.auth_token(&server);
    own_library(&server, "quota-default");
    // No per-user override: the instance-wide default must be the fallback.
    {
        let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
        db.conn
            .execute(
                "UPDATE instance_config SET default_max_user_bytes = 10 WHERE id = 1",
                [],
            )
            .expect("set instance default");
    }

    let op = encrypted_op(&device, &server);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        &token,
        json!({ "ops": [op] }),
    ));

    assert_eq!(
        status,
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "instance default ceiling must apply to owned accounts; body: {body}"
    );
}

#[test]
fn unowned_library_is_exempt_from_quota() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "quota-exempt", "device-a", None);
    let token = device.auth_token(&server);
    // Library is left unowned (self-hosted open-relay model). Even a tiny
    // instance-wide ceiling must not apply.
    {
        let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
        db.conn
            .execute(
                "UPDATE instance_config SET default_max_user_bytes = 1 WHERE id = 1",
                [],
            )
            .expect("set instance default");
    }

    let op = encrypted_op(&device, &server);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        &token,
        json!({ "ops": [op] }),
    ));

    assert!(
        status.is_success(),
        "unowned (self-hosted) library must be exempt from quotas; status {status}, body: {body}"
    );
    assert_eq!(
        stored_op_count(&server, "quota-exempt"),
        1,
        "the accepted op should be persisted"
    );
}
