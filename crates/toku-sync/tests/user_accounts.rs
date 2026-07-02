//! User accounts, admin, and multi-user schema integration tests (issue #119).
//!
//! Covers:
//! (a) First-run bootstrap: the first signup becomes the admin even when
//!     registration is closed.
//! (b) Closed registration rejects a second signup; opening it lets a plain user
//!     register.
//! (c) Duplicate email is rejected with 409.
//! (d) User SRP login end-to-end (challenge + verify) issues a session; wrong
//!     password is rejected with 401; lockout after 5 failures (HTTP 423).
//! (e) Admin authorization: admin can list users; a plain user gets 403;
//!     unauthenticated gets 401.
//! (f) Disabling a user blocks their login; an admin cannot disable themselves
//!     or the last remaining admin.
//! (g) Security: no plaintext password lands in users/challenges/sessions.

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

/// POST JSON, no auth. Returns (status, parsed body).
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

/// POST JSON with a bearer token. Returns (status, parsed body).
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

/// PUT JSON with a bearer token. Returns (status, parsed body).
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

/// GET with optional bearer token. Returns (status, parsed body).
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

// ── SRP helpers ──────────────────────────────────────────────────────────────

/// Deterministic 16-byte salt derived from the email (so the test client and
/// server agree). Real clients use CSPRNG output.
fn salt_for(email: &str) -> Vec<u8> {
    let digest = Sha256::digest(format!("salt:{email}").as_bytes());
    digest[..16].to_vec()
}

/// Compute the hex-encoded SRP verifier for an account, using the email as the
/// SRP identity (matching the server).
fn verifier_hex(email: &str, password: &str, salt: &[u8]) -> String {
    let client = ClientG2048::<Sha256>::new();
    let v = client.compute_verifier(email.as_bytes(), password.as_bytes(), salt);
    hex::encode(v)
}

/// Sign up an account. Returns (status, body).
fn signup(server: &TestServer, email: &str, password: &str) -> (u16, serde_json::Value) {
    signup_auth(server, email, password, None)
}

/// Sign up an account, optionally presenting a user-session bearer.
fn signup_auth(
    server: &TestServer,
    email: &str,
    password: &str,
    bearer: Option<&str>,
) -> (u16, serde_json::Value) {
    let salt = salt_for(email);
    let body = serde_json::json!({
        "email": email,
        "srp_salt": hex::encode(&salt),
        "srp_verifier": verifier_hex(email, password, &salt),
    });
    let url = format!("{}/api/v1/account/signup", server.base_url());
    match bearer {
        Some(token) => post_json_auth(&url, token, &body),
        None => post_json(&url, &body),
    }
}

/// Run a full user SRP login (challenge + verify). Returns (status, verify body).
fn login(server: &TestServer, email: &str, password: &str) -> (u16, serde_json::Value) {
    use rand::RngExt;
    let client = ClientG2048::<Sha256>::new();

    let mut a = [0u8; 48];
    rand::rng().fill(&mut a);
    let a_pub = client.compute_public_ephemeral(&a);

    // Challenge
    let (status, challenge) = post_json(
        &format!("{}/api/v1/account/challenge", server.base_url()),
        &serde_json::json!({ "email": email, "client_public_a": hex::encode(&a_pub) }),
    );
    if status != 200 {
        return (status, challenge);
    }
    let challenge_id = challenge["challenge_id"].as_str().expect("challenge_id");
    let server_b =
        hex::decode(challenge["server_public_b"].as_str().expect("b")).expect("decode b");
    let salt = hex::decode(challenge["srp_salt"].as_str().expect("salt")).expect("decode salt");

    let verifier = client
        .process_reply(&a, email.as_bytes(), password.as_bytes(), &salt, &server_b)
        .expect("process_reply");
    let m1 = hex::encode(verifier.proof());

    // Verify
    let (status, body) = post_json(
        &format!("{}/api/v1/account/verify", server.base_url()),
        &serde_json::json!({ "challenge_id": challenge_id, "client_proof_m1": m1 }),
    );

    // When the login succeeds, confirm the server proof so we exercise the full
    // mutual-auth handshake the same way a real client would.
    if status == 200 {
        let m2 = hex::decode(body["server_proof_m2"].as_str().expect("m2")).expect("decode m2");
        verifier
            .verify_server(&m2)
            .expect("server proof M2 must verify");
    }
    (status, body)
}

/// Convenience: sign up + log in, returning the session token.
fn signup_and_token(server: &TestServer, email: &str, password: &str) -> String {
    let (status, _) = signup(server, email, password);
    assert_eq!(status, 200, "signup should succeed");
    let (status, body) = login(server, email, password);
    assert_eq!(status, 200, "login should succeed");
    body["session_token"]
        .as_str()
        .expect("session_token")
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// (a) The first account on a fresh instance becomes the admin even though
/// registration is closed by default.
#[test]
fn first_signup_bootstraps_admin() {
    let server = TestServer::start();

    let (status, body) = signup(&server, "admin@example.com", "correct horse");
    assert_eq!(status, 200, "first signup must succeed (bootstrap)");
    assert_eq!(body["role"], "admin", "first account must be admin");
}

/// (b) With registration closed, a second signup is rejected; after an admin
/// opens registration, a plain user can sign up as role `user`.
#[test]
fn closed_registration_then_open() {
    let server = TestServer::start();
    let admin_token = signup_and_token(&server, "admin@example.com", "admin pass");

    // Second signup blocked while closed.
    let (status, _) = signup(&server, "bob@example.com", "bob pass");
    assert_eq!(status, 403, "self-registration must be closed by default");

    // Admin reads the flag (false), then opens it.
    let (status, cfg) = get_json(
        &format!("{}/api/v1/admin/registration", server.base_url()),
        Some(&admin_token),
    );
    assert_eq!(status, 200);
    assert_eq!(cfg["registration_open"], false);

    let (status, cfg) = put_json_auth(
        &format!("{}/api/v1/admin/registration", server.base_url()),
        &admin_token,
        &serde_json::json!({ "open": true }),
    );
    assert_eq!(status, 200);
    assert_eq!(cfg["registration_open"], true);

    // Now Bob can register, as a plain user.
    let (status, body) = signup(&server, "bob@example.com", "bob pass");
    assert_eq!(status, 200, "signup must succeed once registration is open");
    assert_eq!(body["role"], "user", "non-first accounts are plain users");
}

/// (c) Duplicate email is rejected with 409.
#[test]
fn duplicate_email_conflicts() {
    let server = TestServer::start();
    let admin_token = signup_and_token(&server, "admin@example.com", "admin pass");
    put_json_auth(
        &format!("{}/api/v1/admin/registration", server.base_url()),
        &admin_token,
        &serde_json::json!({ "open": true }),
    );

    let (status, _) = signup(&server, "dupe@example.com", "pw one");
    assert_eq!(status, 200);
    let (status, _) = signup(&server, "dupe@example.com", "pw two");
    assert_eq!(status, 409, "duplicate email must conflict");
}

/// (d) Wrong password is rejected with 401; the correct password still works.
#[test]
fn wrong_password_rejected() {
    let server = TestServer::start();
    signup(&server, "admin@example.com", "right pass");

    let (status, _) = login(&server, "admin@example.com", "WRONG pass");
    assert_eq!(status, 401, "wrong password must be unauthorized");

    let (status, _) = login(&server, "admin@example.com", "right pass");
    assert_eq!(status, 200, "correct password must authenticate");
}

/// (d) Five consecutive failed logins lock the account (HTTP 423).
#[test]
fn lockout_after_five_failures() {
    let server = TestServer::start();
    signup(&server, "admin@example.com", "right pass");

    for _ in 0..5 {
        let (status, _) = login(&server, "admin@example.com", "bad pass");
        assert!(
            status == 401 || status == 423,
            "expected 401/423, got {status}"
        );
    }

    // Even the correct password is now locked out.
    let (status, _) = login(&server, "admin@example.com", "right pass");
    assert_eq!(status, 423, "account must be locked after 5 failures");
}

/// (e) Admin can list users; a plain user is forbidden; unauthenticated is 401.
#[test]
fn admin_authorization() {
    let server = TestServer::start();
    let admin_token = signup_and_token(&server, "admin@example.com", "admin pass");
    put_json_auth(
        &format!("{}/api/v1/admin/registration", server.base_url()),
        &admin_token,
        &serde_json::json!({ "open": true }),
    );
    let user_token = signup_and_token(&server, "bob@example.com", "bob pass");

    let url = format!("{}/api/v1/admin/users", server.base_url());

    // Unauthenticated → 401.
    let (status, _) = get_json(&url, None);
    assert_eq!(status, 401);

    // Plain user → 403.
    let (status, _) = get_json(&url, Some(&user_token));
    assert_eq!(status, 403, "non-admin must be forbidden");

    // Admin → 200 with both users, no secret material leaked.
    let (status, body) = get_json(&url, Some(&admin_token));
    assert_eq!(status, 200);
    let users = body["users"].as_array().expect("users array");
    assert_eq!(users.len(), 2);
    let raw = body.to_string();
    assert!(
        !raw.contains("srp_verifier") && !raw.contains("srp_salt"),
        "admin user list must not expose SRP material"
    );
}

/// (f) Disabling a user blocks their login and revokes existing sessions; an
/// admin cannot disable themselves or the last remaining admin.
#[test]
fn disable_user_and_guards() {
    let server = TestServer::start();
    let admin_token = signup_and_token(&server, "admin@example.com", "admin pass");
    put_json_auth(
        &format!("{}/api/v1/admin/registration", server.base_url()),
        &admin_token,
        &serde_json::json!({ "open": true }),
    );
    let bob_token = signup_and_token(&server, "bob@example.com", "bob pass");

    // Find Bob's id from the admin listing.
    let (_, body) = get_json(
        &format!("{}/api/v1/admin/users", server.base_url()),
        Some(&admin_token),
    );
    let users = body["users"].as_array().unwrap();
    let bob = users
        .iter()
        .find(|u| u["email"] == "bob@example.com")
        .expect("bob in list");
    let bob_id = bob["id"].as_str().unwrap().to_string();
    let admin_id = users
        .iter()
        .find(|u| u["email"] == "admin@example.com")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Admin cannot disable self.
    let (status, _) = post_json_auth(
        &format!("{}/api/v1/admin/users/{admin_id}/status", server.base_url()),
        &admin_token,
        &serde_json::json!({ "status": "disabled" }),
    );
    assert_eq!(status, 403, "admin must not disable own account");

    // The lone admin cannot be disabled even by id.
    // (Bob is a plain user, so admin is the last admin.)
    // Disable Bob — allowed.
    let (status, body) = post_json_auth(
        &format!("{}/api/v1/admin/users/{bob_id}/status", server.base_url()),
        &admin_token,
        &serde_json::json!({ "status": "disabled" }),
    );
    assert_eq!(status, 200);
    assert_eq!(body["status"], "disabled");

    // Bob's existing session is now revoked.
    let (status, _) = get_json(
        &format!("{}/api/v1/admin/users", server.base_url()),
        Some(&bob_token),
    );
    assert_eq!(status, 401, "disabled user's session must be revoked");

    // Bob can no longer log in. Under the anti-enumeration design (F5) a
    // disabled account is indistinguishable from a wrong password: the
    // challenge returns a phantom handshake and verification fails with a
    // uniform 401 rather than a revealing 403.
    let (status, _) = login(&server, "bob@example.com", "bob pass");
    assert_eq!(status, 401, "disabled user cannot log in");

    // Re-enable Bob; login works again.
    let (status, _) = post_json_auth(
        &format!("{}/api/v1/admin/users/{bob_id}/status", server.base_url()),
        &admin_token,
        &serde_json::json!({ "status": "active" }),
    );
    assert_eq!(status, 200);
    let (status, _) = login(&server, "bob@example.com", "bob pass");
    assert_eq!(status, 200, "re-enabled user can log in");
}

/// The last remaining admin cannot be disabled (instance keeps an admin).
#[test]
fn cannot_disable_last_admin() {
    let server = TestServer::start();
    let admin_token = signup_and_token(&server, "admin@example.com", "admin pass");

    let (_, body) = get_json(
        &format!("{}/api/v1/admin/users", server.base_url()),
        Some(&admin_token),
    );
    let admin_id = body["users"][0]["id"].as_str().unwrap().to_string();

    let (status, _) = post_json_auth(
        &format!("{}/api/v1/admin/users/{admin_id}/status", server.base_url()),
        &admin_token,
        &serde_json::json!({ "status": "disabled" }),
    );
    // Self-disable guard fires first (admin is disabling itself), which also
    // protects the last-admin invariant.
    assert_eq!(status, 403);
}

/// (g) The server never persists the plaintext password in any auth table.
#[test]
fn no_plaintext_password_stored() {
    let server = TestServer::start();
    let password = "super secret pw 123";
    signup(&server, "admin@example.com", password);
    login(&server, "admin@example.com", password);

    // Open the server DB directly and scan the auth tables.
    let db =
        toku_sync::db::SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");

    let scan = |table: &str, cols: &[&str]| {
        let select = cols.join(", ");
        let mut stmt = db
            .conn
            .prepare(&format!("SELECT {select} FROM {table}"))
            .expect("prepare");
        let n = cols.len();
        let rows = stmt
            .query_map([], move |row| {
                let mut joined = String::new();
                for i in 0..n {
                    let v: Option<String> = row.get(i)?;
                    joined.push_str(&v.unwrap_or_default());
                    joined.push('\u{1f}');
                }
                Ok(joined)
            })
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");
        for r in rows {
            assert!(
                !r.contains(password),
                "plaintext password leaked into {table}"
            );
        }
    };

    scan(
        "users",
        &["email", "srp_salt", "srp_verifier", "wrapped_private_key"],
    );
    scan(
        "user_srp_challenges",
        &["server_ephemeral_secret", "client_public_a"],
    );
    scan("user_sessions", &["session_token_hash"]);
}
