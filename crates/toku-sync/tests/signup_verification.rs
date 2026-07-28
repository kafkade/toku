//! Self-serve signup email-verification tests (issue #206, ADR-014 D4).
//!
//! When an instance turns on `require_email_verification`, a new non-admin
//! signup starts unverified and cannot obtain a session until it confirms its
//! address via the emailed token. The admin bootstrap is always auto-verified,
//! and with the flag off the flow is byte-for-byte the pre-existing behaviour.

mod harness;

use harness::TestServer;
use serde_json::json;
use sha2::{Digest, Sha256};
use srp::ClientG2048;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;
use toku_sync::ManagedRuntime;
use toku_sync::db::SyncDatabase;
use toku_sync::mailer::CapturingMailer;

fn rt() -> Runtime {
    Runtime::new().expect("tokio runtime")
}

async fn post(
    base_url: &str,
    path: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    let resp = reqwest::Client::new()
        .post(format!("{base_url}{path}"))
        .json(&body)
        .send()
        .await
        .expect("request sent");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

// ── SRP helpers ───────────────────────────────────────────────────────────────

fn salt_for(email: &str) -> Vec<u8> {
    Sha256::digest(format!("salt:{email}").as_bytes())[..16].to_vec()
}

fn verifier_hex(email: &str, password: &str, salt: &[u8]) -> String {
    let client = ClientG2048::<Sha256>::new();
    hex::encode(client.compute_verifier(email.as_bytes(), password.as_bytes(), salt))
}

fn signup(server: &TestServer, email: &str, password: &str) -> (u16, serde_json::Value) {
    let salt = salt_for(email);
    let body = json!({
        "email": email,
        "srp_salt": hex::encode(&salt),
        "srp_verifier": verifier_hex(email, password, &salt),
    });
    let (status, text) = rt().block_on(post(server.base_url(), "/api/v1/account/signup", body));
    (
        status.as_u16(),
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
    )
}

/// Attempt an SRP login; returns the HTTP status (200 = session issued).
fn login_status(server: &TestServer, email: &str, password: &str) -> u16 {
    use rand::RngExt;
    let client = ClientG2048::<Sha256>::new();
    let mut a = [0u8; 48];
    rand::rng().fill(&mut a);
    let a_pub = client.compute_public_ephemeral(&a);

    let (status, challenge) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/challenge",
        json!({ "email": email, "client_public_a": hex::encode(&a_pub) }),
    ));
    if status != reqwest::StatusCode::OK {
        return status.as_u16();
    }
    let challenge: serde_json::Value = serde_json::from_str(&challenge).unwrap();
    let challenge_id = challenge["challenge_id"].as_str().unwrap();
    let server_b = hex::decode(challenge["server_public_b"].as_str().unwrap()).unwrap();
    let salt = hex::decode(challenge["srp_salt"].as_str().unwrap()).unwrap();
    let verifier = client
        .process_reply(&a, email.as_bytes(), password.as_bytes(), &salt, &server_b)
        .expect("process_reply");
    let m1 = hex::encode(verifier.proof());

    let (status, _) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/verify",
        json!({ "challenge_id": challenge_id, "client_proof_m1": m1 }),
    ));
    status.as_u16()
}

/// Turn on `require_email_verification` on a running server's database and open
/// registration so a second (non-admin) account can sign up.
fn enable_verification_and_registration(server: &TestServer) {
    let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
    db.conn
        .execute(
            "UPDATE instance_config
             SET require_email_verification = 1, registration_open = 1 WHERE id = 1",
            [],
        )
        .expect("enable verification");
}

/// Extract the `token` query parameter from a captured verification URL.
fn token_from_url(url: &str) -> String {
    url.split("token=")
        .nth(1)
        .expect("verify url carries a token")
        .to_string()
}

fn managed_with(mailer: CapturingMailer) -> ManagedRuntime {
    ManagedRuntime::new(
        Arc::new(mailer),
        Some("https://sync.test".to_string()),
        0,
        Duration::from_secs(60),
    )
}

#[test]
fn unverified_user_cannot_log_in_until_verified() {
    let mailer = CapturingMailer::new();
    let server = TestServer::start_managed(managed_with(mailer.clone()));

    // Admin bootstrap: always auto-verified, and can log in immediately even
    // with the flag on.
    let (status, admin_body) = signup(&server, "admin@example.com", "correct horse");
    assert_eq!(status, 200, "admin signup should succeed");
    assert_eq!(
        admin_body["email_verification_required"], false,
        "admin bootstrap is never gated on verification"
    );
    assert_eq!(
        login_status(&server, "admin@example.com", "correct horse"),
        200,
        "admin can log in without verifying"
    );

    // Now require verification for subsequent signups.
    enable_verification_and_registration(&server);

    let (status, body) = signup(&server, "user@example.com", "hunter2 hunter2");
    assert_eq!(status, 200, "user signup should still succeed");
    assert_eq!(
        body["email_verification_required"], true,
        "the new user must be told verification is required"
    );

    // Before verifying, login is blocked with 403 even with the right password.
    assert_eq!(
        login_status(&server, "user@example.com", "hunter2 hunter2"),
        403,
        "unverified user must be blocked from logging in"
    );

    // The mailer captured a verification link; redeem its token.
    let url = mailer.last_url().expect("a verification email was sent");
    let token = token_from_url(&url);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/verify-email",
        json!({ "token": token }),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "verify-email failed: {body}"
    );

    // After verifying, login succeeds.
    assert_eq!(
        login_status(&server, "user@example.com", "hunter2 hunter2"),
        200,
        "verified user can log in"
    );
}

#[test]
fn verification_disabled_by_default() {
    // Default runtime + default instance config = no verification gate.
    let server = TestServer::start();
    let (status, body) = signup(&server, "admin@example.com", "correct horse");
    assert_eq!(status, 200);
    assert_eq!(
        body["email_verification_required"], false,
        "verification must be off by default (self-hosted)"
    );
    assert_eq!(
        login_status(&server, "admin@example.com", "correct horse"),
        200,
        "login works without any verification step by default"
    );
}

#[test]
fn invalid_verification_token_is_rejected() {
    let mailer = CapturingMailer::new();
    let server = TestServer::start_managed(managed_with(mailer));
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/verify-email",
        json!({ "token": "not-a-real-token" }),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "an unknown token must be rejected; body: {body}"
    );
}

#[test]
fn resend_issues_a_working_verification_token() {
    let mailer = CapturingMailer::new();
    let server = TestServer::start_managed(managed_with(mailer.clone()));

    // Admin bootstrap (auto-verified), then require verification for the next user.
    let (status, _) = signup(&server, "admin@example.com", "correct horse");
    assert_eq!(status, 200);
    enable_verification_and_registration(&server);

    let (status, _) = signup(&server, "user@example.com", "hunter2 hunter2");
    assert_eq!(status, 200);
    assert_eq!(
        login_status(&server, "user@example.com", "hunter2 hunter2"),
        403,
        "unverified user starts blocked"
    );

    // Ask for a fresh verification email and redeem *that* token.
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/resend-verification",
        json!({ "email": "user@example.com" }),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "resend should be accepted: {body}"
    );

    let url = mailer.last_url().expect("resend sent a verification email");
    let token = token_from_url(&url);
    let (status, body) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/verify-email",
        json!({ "token": token }),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "a resent token must verify the account: {body}"
    );
    assert_eq!(
        login_status(&server, "user@example.com", "hunter2 hunter2"),
        200,
        "the account can log in after redeeming a resent token"
    );
}

#[test]
fn resend_is_velocity_capped_per_email() {
    let mailer = CapturingMailer::new();
    let server = TestServer::start_managed(managed_with(mailer));

    // The velocity cap applies per email regardless of whether the account
    // exists (no existence leak), so an unknown address exercises it directly.
    let email = "flooder@example.com";
    for attempt in 0..3 {
        let (status, body) = rt().block_on(post(
            server.base_url(),
            "/api/v1/account/resend-verification",
            json!({ "email": email }),
        ));
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "attempt {attempt} within the window should be accepted: {body}"
        );
    }

    // The next attempt inside the same window trips the per-email cap.
    let (status, _) = rt().block_on(post(
        server.base_url(),
        "/api/v1/account/resend-verification",
        json!({ "email": email }),
    ));
    assert_eq!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "a 4th resend inside the window must be rate-limited"
    );
}
