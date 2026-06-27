//! Authenticated, Secret-Key-gated device enrollment integration tests (issue #120).
//!
//! Covers:
//! (a) Open `register` stays available on a fresh (zero-account) instance, but is
//!     hard-gated (403) once an account exists — guessing a `library_id` no longer
//!     grants access.
//! (b) Device enrollment requires a valid account session (401 without one).
//! (c) A successful enrollment binds the device to the user's library and issues a
//!     device session token that can push/pull.
//! (d) Enrolling into another user's library is forbidden (403).
//! (e) Optional approval flow: a second device on a library with approvals enabled
//!     is held `pending`, cannot mint a session, and only syncs after an existing
//!     trusted device (the account owner) approves it; rejection blocks it.
//! (f) Account-scoped device listing/deregistration never crosses user boundaries.

mod harness;

use harness::TestServer;
use sha2::{Digest, Sha256};
use srp::ClientG2048;

// ── HTTP helpers ─────────────────────────────────────────────────────────────

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build rt")
}

fn post_json(url: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    rt().block_on(async {
        let resp = reqwest::Client::new()
            .post(url)
            .json(body)
            .send()
            .await
            .expect("send request");
        let status = resp.status().as_u16();
        let val: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, val)
    })
}

fn post_json_auth(url: &str, bearer: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    rt().block_on(async {
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .expect("send request");
        let status = resp.status().as_u16();
        let val: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, val)
    })
}

fn put_json_auth(url: &str, bearer: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    rt().block_on(async {
        let resp = reqwest::Client::new()
            .put(url)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .expect("send request");
        let status = resp.status().as_u16();
        let val: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, val)
    })
}

/// GET with an optional bearer token.
fn get_json(url: &str, bearer: Option<&str>) -> (u16, serde_json::Value) {
    rt().block_on(async {
        let mut req = reqwest::Client::new().get(url);
        if let Some(token) = bearer {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await.expect("send request");
        let status = resp.status().as_u16();
        let val: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, val)
    })
}

/// DELETE with an optional bearer token.
fn delete_json(url: &str, bearer: Option<&str>) -> (u16, serde_json::Value) {
    rt().block_on(async {
        let mut req = reqwest::Client::new().delete(url);
        if let Some(token) = bearer {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await.expect("send request");
        let status = resp.status().as_u16();
        let val: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, val)
    })
}

// ── Account SRP helpers (mirror the server's email-keyed SRP) ─────────────────

fn salt_for(email: &str) -> Vec<u8> {
    Sha256::digest(format!("salt:{email}").as_bytes())[..16].to_vec()
}

fn verifier_hex(email: &str, password: &str, salt: &[u8]) -> String {
    let client = ClientG2048::<Sha256>::new();
    hex::encode(client.compute_verifier(email.as_bytes(), password.as_bytes(), salt))
}

/// Sign up an account (no auth). Returns (status, body).
fn signup(server: &TestServer, email: &str, password: &str) -> (u16, serde_json::Value) {
    let salt = salt_for(email);
    let body = serde_json::json!({
        "email": email,
        "srp_salt": hex::encode(&salt),
        "srp_verifier": verifier_hex(email, password, &salt),
    });
    post_json(
        &format!("{}/api/v1/account/signup", server.base_url()),
        &body,
    )
}

/// Full account SRP login. Returns the user-session token.
fn login_token(server: &TestServer, email: &str, password: &str) -> String {
    use rand::RngExt;
    let client = ClientG2048::<Sha256>::new();

    let mut a = [0u8; 48];
    rand::rng().fill(&mut a);
    let a_pub = client.compute_public_ephemeral(&a);

    let (status, challenge) = post_json(
        &format!("{}/api/v1/account/challenge", server.base_url()),
        &serde_json::json!({ "email": email, "client_public_a": hex::encode(&a_pub) }),
    );
    assert_eq!(status, 200, "account challenge failed: {challenge}");
    let challenge_id = challenge["challenge_id"].as_str().expect("challenge_id");
    let server_b = hex::decode(challenge["server_public_b"].as_str().unwrap()).unwrap();
    let salt = hex::decode(challenge["srp_salt"].as_str().unwrap()).unwrap();

    let verifier = client
        .process_reply(&a, email.as_bytes(), password.as_bytes(), &salt, &server_b)
        .expect("process_reply");
    let m1 = hex::encode(verifier.proof());

    let (status, body) = post_json(
        &format!("{}/api/v1/account/verify", server.base_url()),
        &serde_json::json!({ "challenge_id": challenge_id, "client_proof_m1": m1 }),
    );
    assert_eq!(status, 200, "account verify failed: {body}");
    body["session_token"].as_str().unwrap().to_string()
}

/// Sign up + log in, returning the user-session token.
fn account(server: &TestServer, email: &str, password: &str) -> String {
    let (status, body) = signup(server, email, password);
    assert_eq!(status, 200, "signup failed: {body}");
    login_token(server, email, password)
}

/// Open self-registration so a second account can be created.
fn open_registration(server: &TestServer, admin_token: &str) {
    let (status, _) = put_json_auth(
        &format!("{}/api/v1/admin/registration", server.base_url()),
        admin_token,
        &serde_json::json!({ "open": true }),
    );
    assert_eq!(status, 200);
}

/// Enroll a device under `user_token`. Returns (status, body).
fn enroll(
    server: &TestServer,
    user_token: &str,
    library_id: Option<&str>,
    device_name: &str,
) -> (u16, serde_json::Value) {
    let mut body = serde_json::json!({ "device_name": device_name });
    if let Some(lib) = library_id {
        body["library_id"] = serde_json::Value::String(lib.to_string());
    }
    post_json_auth(
        &format!("{}/api/v1/devices/enroll", server.base_url()),
        user_token,
        &body,
    )
}

/// Push a single op as a device, then pull it back. Returns the number of ops
/// the device can see — proves the device session token authenticates sync.
fn push_then_pull_count(server: &TestServer, device_token: &str, op_id: &str) -> usize {
    let (status, _) = post_json_auth(
        &format!("{}/api/v1/push", server.base_url()),
        device_token,
        &serde_json::json!({
            "ops": [{
                "op_id": op_id,
                "device_id": "dev",
                "hlc": "2026-06-01T00:00:00.000Z-0001-dev",
                "entity_type": "book",
                "entity_id": "book-1",
                "op_type": "create",
                "payload": { "title": "ciphertext" }
            }]
        }),
    );
    assert_eq!(status, 200, "push should succeed for an active device");

    let (status, pull) = get_json(
        &format!("{}/api/v1/pull", server.base_url()),
        Some(device_token),
    );
    assert_eq!(status, 200, "pull should succeed for an active device");
    pull["ops"].as_array().map(|a| a.len()).unwrap_or(0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// (a) Open `register` works before any account exists, but is hard-gated once
/// the instance is account-managed — guessing a `library_id` no longer works.
#[test]
fn register_open_until_first_account_then_gated() {
    let server = TestServer::start();

    // Fresh instance: legacy passwordless register still works (bootstrap/relay).
    let (status, body) = post_json(
        &format!("{}/api/v1/register", server.base_url()),
        &serde_json::json!({ "library_id": "legacy-lib", "device_name": "relay" }),
    );
    assert_eq!(
        status, 200,
        "register must be open on a zero-account instance"
    );
    assert!(!body["auth_token"].as_str().unwrap().is_empty());

    // Create the first account → instance is now managed.
    account(&server, "admin@example.com", "admin pass");

    // Guessing any library_id via the open path is now rejected.
    let (status, _) = post_json(
        &format!("{}/api/v1/register", server.base_url()),
        &serde_json::json!({ "library_id": "victim-lib", "device_name": "attacker" }),
    );
    assert_eq!(
        status, 403,
        "open register must be closed on an account-managed instance"
    );
}

/// (b) Device enrollment requires a valid account session.
#[test]
fn enrollment_requires_account_auth() {
    let server = TestServer::start();
    account(&server, "admin@example.com", "admin pass");

    // No bearer → 401.
    let (status, _) = post_json(
        &format!("{}/api/v1/devices/enroll", server.base_url()),
        &serde_json::json!({ "device_name": "laptop" }),
    );
    assert_eq!(status, 401, "enroll without a session must be unauthorized");

    // Bogus bearer → 401.
    let (status, _) = enroll(&server, "not-a-real-token", None, "laptop");
    assert_eq!(
        status, 401,
        "enroll with an invalid session must be rejected"
    );
}

/// (c) A successful enrollment issues a device session token that can sync, and
/// binds the device to a library owned by the account.
#[test]
fn enrolled_device_can_sync() {
    let server = TestServer::start();
    let token = account(&server, "admin@example.com", "admin pass");

    let (status, body) = enroll(&server, &token, None, "laptop");
    assert_eq!(status, 200, "enrollment should succeed: {body}");
    assert_eq!(body["status"], "active");
    let device_token = body["session_token"].as_str().expect("session_token");
    assert!(!body["library_id"].as_str().unwrap().is_empty());

    assert_eq!(
        push_then_pull_count(&server, device_token, "op-1"),
        1,
        "the enrolled device must be able to push and pull"
    );

    // A guessed/garbage device token cannot sync.
    let (status, _) = get_json(
        &format!("{}/api/v1/pull", server.base_url()),
        Some("garbage-device-token"),
    );
    assert_eq!(status, 401);
}

/// (d) Enrolling into a library owned by a different account is forbidden.
#[test]
fn cannot_enroll_into_foreign_library() {
    let server = TestServer::start();
    let admin = account(&server, "admin@example.com", "admin pass");
    open_registration(&server, &admin);
    let bob = account(&server, "bob@example.com", "bob pass");

    // Admin creates a library by enrolling their first device.
    let (status, body) = enroll(&server, &admin, None, "admin-laptop");
    assert_eq!(status, 200);
    let admin_lib = body["library_id"].as_str().unwrap().to_string();

    // Bob tries to enroll into the admin's library → 403.
    let (status, _) = enroll(&server, &bob, Some(&admin_lib), "bob-intruder");
    assert_eq!(
        status, 403,
        "must not enroll into another account's library"
    );
}

/// (e) Approval flow: with approvals enabled, the second device on a library is
/// held pending, cannot mint a session, and only syncs after the owner approves.
#[test]
fn approval_flow_pending_then_approved() {
    let server = TestServer::start();
    let token = account(&server, "admin@example.com", "admin pass");

    // Enable the device-approval gate.
    let (status, cfg) = put_json_auth(
        &format!("{}/api/v1/admin/device-approvals", server.base_url()),
        &token,
        &serde_json::json!({ "required": true }),
    );
    assert_eq!(status, 200);
    assert_eq!(cfg["device_approvals_required"], true);

    // First device: still active (nothing to approve against) + can sync.
    let (status, dev_a) = enroll(&server, &token, None, "device-a");
    assert_eq!(status, 200);
    assert_eq!(dev_a["status"], "active");
    let lib = dev_a["library_id"].as_str().unwrap().to_string();
    let a_token = dev_a["session_token"].as_str().unwrap().to_string();
    assert_eq!(push_then_pull_count(&server, &a_token, "op-a"), 1);

    // Second device into the same library: pending, no token.
    let (status, dev_b) = enroll(&server, &token, Some(&lib), "device-b");
    assert_eq!(status, 200);
    assert_eq!(dev_b["status"], "pending");
    assert!(dev_b["session_token"].is_null());
    let b_id = dev_b["device_id"].as_str().unwrap().to_string();

    // Pending device cannot mint a session yet.
    let (status, _) = post_json_auth(
        &format!("{}/api/v1/devices/{b_id}/session", server.base_url()),
        &token,
        &serde_json::json!({}),
    );
    assert_eq!(status, 403, "pending device must not get a session token");

    // Owner approves device B.
    let (status, approved) = post_json_auth(
        &format!("{}/api/v1/devices/{b_id}/approval", server.base_url()),
        &token,
        &serde_json::json!({ "decision": "approve" }),
    );
    assert_eq!(status, 200, "approval should succeed: {approved}");
    assert_eq!(approved["status"], "active");

    // Now device B can mint a session and pull the existing op.
    let (status, sess) = post_json_auth(
        &format!("{}/api/v1/devices/{b_id}/session", server.base_url()),
        &token,
        &serde_json::json!({}),
    );
    assert_eq!(status, 200, "approved device must get a session: {sess}");
    let b_token = sess["session_token"].as_str().unwrap();
    let (status, pull) = get_json(&format!("{}/api/v1/pull", server.base_url()), Some(b_token));
    assert_eq!(status, 200);
    assert_eq!(pull["ops"].as_array().unwrap().len(), 1);
}

/// (e) A rejected device is permanently blocked from minting a session.
#[test]
fn approval_flow_rejection_blocks_device() {
    let server = TestServer::start();
    let token = account(&server, "admin@example.com", "admin pass");
    put_json_auth(
        &format!("{}/api/v1/admin/device-approvals", server.base_url()),
        &token,
        &serde_json::json!({ "required": true }),
    );

    let (_, dev_a) = enroll(&server, &token, None, "device-a");
    let lib = dev_a["library_id"].as_str().unwrap().to_string();
    let (_, dev_b) = enroll(&server, &token, Some(&lib), "device-b");
    let b_id = dev_b["device_id"].as_str().unwrap().to_string();

    let (status, rejected) = post_json_auth(
        &format!("{}/api/v1/devices/{b_id}/approval", server.base_url()),
        &token,
        &serde_json::json!({ "decision": "reject" }),
    );
    assert_eq!(status, 200);
    assert_eq!(rejected["status"], "rejected");

    let (status, _) = post_json_auth(
        &format!("{}/api/v1/devices/{b_id}/session", server.base_url()),
        &token,
        &serde_json::json!({}),
    );
    assert_eq!(status, 403, "rejected device must never get a session");
}

/// (f) Account device listing/deregistration is strictly scoped to the owner.
#[test]
fn account_device_management_is_user_scoped() {
    let server = TestServer::start();
    let admin = account(&server, "admin@example.com", "admin pass");
    open_registration(&server, &admin);
    let bob = account(&server, "bob@example.com", "bob pass");

    let (_, admin_dev) = enroll(&server, &admin, None, "admin-laptop");
    let admin_device_id = admin_dev["device_id"].as_str().unwrap().to_string();
    enroll(&server, &bob, None, "bob-phone");

    // Each account sees only its own device.
    let (status, admin_list) = get_json(
        &format!("{}/api/v1/account/devices", server.base_url()),
        Some(&admin),
    );
    assert_eq!(status, 200);
    let admin_devices = admin_list["devices"].as_array().unwrap();
    assert_eq!(admin_devices.len(), 1);
    assert_eq!(admin_devices[0]["device_name"], "admin-laptop");

    let (_, bob_list) = get_json(
        &format!("{}/api/v1/account/devices", server.base_url()),
        Some(&bob),
    );
    assert_eq!(bob_list["devices"].as_array().unwrap().len(), 1);
    assert_eq!(bob_list["devices"][0]["device_name"], "bob-phone");

    // Bob cannot delete the admin's device.
    let (status, _) = delete_json(
        &format!(
            "{}/api/v1/account/devices/{admin_device_id}",
            server.base_url()
        ),
        Some(&bob),
    );
    assert_eq!(
        status, 404,
        "cross-user device deletion must not be allowed"
    );

    // The admin can delete their own device.
    let (status, _) = delete_json(
        &format!(
            "{}/api/v1/account/devices/{admin_device_id}",
            server.base_url()
        ),
        Some(&admin),
    );
    assert_eq!(status, 200);
    let (_, admin_list) = get_json(
        &format!("{}/api/v1/account/devices", server.base_url()),
        Some(&admin),
    );
    assert_eq!(admin_list["devices"].as_array().unwrap().len(), 0);
}
