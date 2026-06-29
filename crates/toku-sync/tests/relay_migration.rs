//! Relay → account migration compatibility tests (issue #126).
//!
//! Seeds an "old" relay database — an unowned (pre-account) library/device plus
//! a plaintext op — then drives `toku sync migrate` end-to-end and asserts:
//! orphans are adopted by the bootstrap admin, single-passphrase data survives,
//! previously-plaintext ops become zero-knowledge ciphertext, and pre-account
//! clients are turned away with HTTP 426 once the instance is migrated.

mod harness;

use harness::{SimulatedDevice, TestServer};
use toku_sync::db::SyncDatabase;

/// Insert a fully-orphaned relay library + device (user_id IS NULL), mimicking a
/// device registered under the old unauthenticated model.
fn seed_orphan_library(server: &TestServer) {
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .execute(
            "INSERT INTO libraries (id, created_at) VALUES ('orphan-lib', datetime('now'))",
            [],
        )
        .expect("insert orphan library");
    db.conn
        .execute(
            "INSERT INTO devices (device_id, library_id, device_name, auth_token_hash, created_at)
             VALUES ('orphan-dev', 'orphan-lib', 'legacy', 'deadbeef', datetime('now'))",
            [],
        )
        .expect("insert orphan device");
}

fn health(server: &TestServer) -> serde_json::Value {
    let url = format!("{}/health", server.base_url());
    let body = reqwest::blocking::get(url).unwrap().text().unwrap();
    serde_json::from_str(&body).unwrap()
}

#[test]
fn migrate_adopts_orphans_rekeys_and_locks_out_old_clients() {
    let server = TestServer::start();
    seed_orphan_library(&server);

    // Fresh instance speaks protocol 2 but accepts legacy clients (min = 1).
    let h = health(&server);
    assert_eq!(h["protocol_version"], 2);
    assert_eq!(h["min_protocol"], 1);

    // A relay device with a single passphrase, holding encrypted ops.
    let mut dev = SimulatedDevice::register(&server, "old-library-id", "laptop", Some("pw"));
    let book = dev.add_book("The Old Book");
    dev.set_rating(book, 9);
    dev.push();

    // Inject a previously-plaintext op directly into the device's library to
    // prove migration re-protects readable data.
    {
        let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
        db.conn
            .execute(
                "INSERT INTO ops (op_id, library_id, device_id, hlc, entity_type, entity_id,
                                  op_type, payload, received_at)
                 VALUES ('plain-op', 'old-library-id', ?1, '999|0', 'book', ?2, 'update',
                         '{\"title\":\"plaintext\"}', datetime('now'))",
                rusqlite::params![
                    dev.device_id().to_string(),
                    uuid::Uuid::now_v7().to_string()
                ],
            )
            .expect("seed plaintext op");
    }

    // Migrate: becomes the bootstrap admin, adopts orphan + own library, rekeys.
    let secret = toku_core::SecretKey::generate().unwrap();
    let outcome = toku_sync_client::migrate(dev.data_dir_path(), "owner@toku.test", "pw", &secret)
        .expect("migrate succeeds");

    assert_eq!(outcome.role, "admin");
    assert!(
        outcome.adopted_libraries >= 2,
        "adopts orphan + device library"
    );
    assert!(outcome.adopted_devices >= 2);
    assert!(
        outcome.ops_replaced >= 3,
        "all ops re-keyed: {}",
        outcome.ops_replaced
    );
    assert!(outcome.had_encryption);

    // Every server op is now zero-knowledge ciphertext (incl. the seeded plaintext).
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    let mut stmt = db.conn.prepare("SELECT payload FROM ops").unwrap();
    let payloads: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!payloads.is_empty());
    for p in &payloads {
        assert!(
            p.contains("\"ev\""),
            "op must be an encrypted envelope: {p}"
        );
        assert!(!p.contains("plaintext"), "no readable content on server");
    }

    // Instance is now locked to protocol 2: old clients get 426.
    assert_eq!(health(&server)["min_protocol"], 2);
    let resp = reqwest::blocking::Client::new()
        .post(format!("{}/api/v1/register", server.base_url()))
        .header("x-toku-sync-protocol", "1")
        .json(&serde_json::json!({"library_id": "x", "device_name": "old"}))
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 426, "pre-account client rejected");
}
