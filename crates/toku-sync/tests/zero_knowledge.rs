//! Zero-knowledge enforcement tests (issue #121).
//!
//! These tests assert the hosted-mode guarantee that the server can never read
//! user content: op payloads and snapshots must be ciphertext, plaintext
//! uploads are rejected, and what the server persists is undecryptable without
//! the client-held key.

mod harness;

use harness::{SimulatedDevice, TestServer};
use serde_json::json;
use tokio::runtime::Runtime;
use toku_core::{EntityType, HybridClock, OpType, SyncKey};
use toku_sync::db::SyncDatabase;
use uuid::Uuid;

fn rt() -> Runtime {
    Runtime::new().expect("tokio runtime")
}

/// POST `body` to `path` with a bearer token; return (status, body text).
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

/// Build a wire op whose payload is `payload` (already-shaped JSON).
fn wire_op(device_id: Uuid, entity_id: Uuid, payload: serde_json::Value) -> serde_json::Value {
    json!({
        "op_id": Uuid::now_v7().to_string(),
        "device_id": device_id.to_string(),
        "hlc": hlc_for(device_id),
        "entity_type": EntityType::Book.as_str(),
        "entity_id": entity_id.to_string(),
        "op_type": OpType::Create.as_str(),
        "payload": payload,
    })
}

#[test]
fn server_rejects_plaintext_op_push() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "zk-plain-op", "device-a", None);
    let token = device.auth_token(&server);

    // A readable `fields` object must be refused.
    let op = wire_op(
        device.device_id(),
        Uuid::now_v7(),
        json!({ "title": "Plaintext Secret" }),
    );
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        &token,
        json!({ "ops": [op] }),
    ));

    assert_eq!(
        status,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY,
        "plaintext push should be rejected (422); body: {body}"
    );
}

#[test]
fn server_accepts_ciphertext_and_stores_it_opaquely() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "zk-cipher-op", "device-a", None);
    let token = device.auth_token(&server);
    let key = device.sync_key(&server);

    let entity_id = Uuid::now_v7();
    let secret_title = "Top Secret Title";
    let envelope = toku_core::encrypt_fields(
        &key,
        &json!({ "title": secret_title }),
        &EntityType::Book,
        &entity_id,
        &OpType::Create,
    )
    .expect("encrypt fields");

    let op = wire_op(
        device.device_id(),
        entity_id,
        serde_json::to_value(&envelope).unwrap(),
    );
    let op_id = op["op_id"].as_str().unwrap().to_string();

    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/push",
        &token,
        json!({ "ops": [op] }),
    ));
    assert!(
        status.is_success(),
        "ciphertext push should succeed; status {status}, body: {body}"
    );

    // White-box: the persisted payload must be ciphertext, never the plaintext.
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    let stored: String = db
        .conn
        .query_row(
            "SELECT payload FROM ops WHERE op_id = ?1",
            [&op_id],
            |row| row.get(0),
        )
        .expect("stored op present");

    assert!(
        !stored.contains(secret_title),
        "server must not store plaintext content; stored: {stored}"
    );
    let stored_value: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert!(
        stored_value.get("ev").is_some() && stored_value.get("ciphertext").is_some(),
        "stored payload must be an encrypted envelope: {stored}"
    );

    // The server holds no key; a different key cannot decrypt the stored blob.
    let stored_envelope: toku_core::EncryptedEnvelope =
        serde_json::from_value(stored_value).unwrap();
    let wrong_key = SyncKey::derive("not-the-real-passphrase", &[7u8; 16]).unwrap();
    assert!(
        toku_core::decrypt_fields(
            &wrong_key,
            &stored_envelope,
            &EntityType::Book,
            &entity_id,
            &OpType::Create,
        )
        .is_err(),
        "stored ciphertext must be undecryptable without the right key"
    );

    // Sanity: the real key still recovers the content (proves it round-trips).
    let recovered = toku_core::decrypt_fields(
        &key,
        &stored_envelope,
        &EntityType::Book,
        &entity_id,
        &OpType::Create,
    )
    .expect("decrypt with real key");
    assert_eq!(recovered["title"], secret_title);
}

#[test]
fn server_rejects_plaintext_snapshot() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "zk-plain-snap", "device-a", None);
    let token = device.auth_token(&server);

    // A raw LibrarySnapshot-shaped JSON blob must be refused.
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/snapshot",
        &token,
        json!({
            "snapshot_json": "{\"version\":1,\"library\":{\"books\":[]}}",
            "hlc_at_snapshot": hlc_for(device.device_id()),
        }),
    ));

    assert_eq!(
        status,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY,
        "plaintext snapshot should be rejected (422); body: {body}"
    );
}

#[test]
fn server_accepts_encrypted_snapshot_and_stores_it_opaquely() {
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "zk-cipher-snap", "device-a", None);
    let token = device.auth_token(&server);
    let key = device.sync_key(&server);

    let secret = "secret-library-contents";
    let plaintext_snapshot = format!("{{\"version\":1,\"marker\":\"{secret}\"}}");
    let envelope =
        toku_core::encrypt_snapshot(&key, &plaintext_snapshot).expect("encrypt snapshot");
    let blob = serde_json::to_string(&envelope).unwrap();

    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/snapshot",
        &token,
        json!({
            "snapshot_json": blob,
            "hlc_at_snapshot": hlc_for(device.device_id()),
        }),
    ));
    assert!(
        status.is_success(),
        "encrypted snapshot upload should succeed; status {status}, body: {body}"
    );

    // White-box: persisted snapshot must be ciphertext, never the plaintext.
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    let stored: String = db
        .conn
        .query_row(
            "SELECT snapshot_json FROM snapshots WHERE library_id = ?1",
            ["zk-cipher-snap"],
            |row| row.get(0),
        )
        .expect("stored snapshot present");

    assert!(
        !stored.contains(secret),
        "server must not store plaintext snapshot content; stored: {stored}"
    );
    let stored_envelope: toku_core::EncryptedEnvelope =
        serde_json::from_str(&stored).expect("stored snapshot is an envelope");
    assert_eq!(
        toku_core::decrypt_snapshot(&key, &stored_envelope).unwrap(),
        plaintext_snapshot,
        "the real key recovers the snapshot"
    );
}
