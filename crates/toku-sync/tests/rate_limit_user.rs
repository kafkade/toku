//! Per-authenticated-user rate limiting tests (issue #206, ADR-014 D3).
//!
//! The managed tier layers a per-user request window *above* the per-IP + global
//! limiter. These tests assert that one user hitting the ceiling is throttled
//! (429) while a second user is unaffected, and that the limiter is disabled by
//! default (self-hosted relay unchanged).

mod harness;

use harness::{SimulatedDevice, TestServer};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;
use toku_sync::ManagedRuntime;
use toku_sync::db::SyncDatabase;
use toku_sync::mailer::LoggingMailer;

fn rt() -> Runtime {
    Runtime::new().expect("tokio runtime")
}

/// GET `path` with a bearer token; return the HTTP status.
fn get_status(base_url: &str, path: &str, token: &str) -> reqwest::StatusCode {
    rt().block_on(async {
        reqwest::Client::new()
            .get(format!("{base_url}{path}"))
            .bearer_auth(token)
            .send()
            .await
            .expect("request sent")
            .status()
    })
}

/// Attach `library_id` to a fresh owning account so per-user limiting engages
/// (unowned libraries are intentionally exempt).
fn own_library(server: &TestServer, library_id: &str) -> String {
    let user_id = uuid::Uuid::now_v7().to_string();
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .execute(
            "INSERT INTO users (id, email, srp_salt, srp_verifier, created_at)
             VALUES (?1, ?2, 'aa', 'bb', datetime('now'))",
            rusqlite::params![user_id, format!("{user_id}@example.com")],
        )
        .expect("insert user");
    db.conn
        .execute(
            "UPDATE libraries SET user_id = ?1 WHERE id = ?2",
            rusqlite::params![user_id, library_id],
        )
        .expect("attach library");
    user_id
}

fn managed_with_user_rate(max: u32) -> ManagedRuntime {
    ManagedRuntime::new(Arc::new(LoggingMailer), None, max, Duration::from_secs(60))
}

#[test]
fn per_user_limit_throttles_one_user_not_another() {
    // Allow 3 requests per user per window.
    let server = TestServer::start_managed(managed_with_user_rate(3));

    let device_a = SimulatedDevice::register(&server, "rl-user-a", "device-a", None);
    let device_b = SimulatedDevice::register(&server, "rl-user-b", "device-b", None);
    own_library(&server, "rl-user-a");
    own_library(&server, "rl-user-b");

    let token_a = device_a.auth_token(&server);
    let token_b = device_b.auth_token(&server);

    // User A: the 4th request within the window must be throttled.
    let mut throttled = false;
    for _ in 0..4 {
        let status = get_status(server.base_url(), "/api/v1/salt", &token_a);
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            throttled = true;
        }
    }
    assert!(throttled, "user A should hit the per-user ceiling");

    // User B shares the server but has its own bucket — unaffected.
    let status_b = get_status(server.base_url(), "/api/v1/salt", &token_b);
    assert_ne!(
        status_b,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "a second user must not be throttled by user A's traffic"
    );
}

#[test]
fn per_user_limit_disabled_by_default() {
    // Default runtime: per-user limiter off. Many authed requests from one user
    // must all pass (the per-IP limiter's default ceiling is far higher).
    let server = TestServer::start();
    let device = SimulatedDevice::register(&server, "rl-off", "device-a", None);
    own_library(&server, "rl-off");
    let token = device.auth_token(&server);

    for _ in 0..10 {
        let status = get_status(server.base_url(), "/api/v1/salt", &token);
        assert_ne!(
            status,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "per-user limiting must be off by default"
        );
    }
}
