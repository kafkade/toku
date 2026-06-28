//! Account key-bundle endpoint tests (issue #143).
//!
//! Covers the zero-knowledge multi-device recovery contract used by #123:
//!
//! (a) Signup persists the opaque `wrapped_data_key`; after SRP login a
//!     `GET /api/v1/account/keys` returns the exact four uploaded strings
//!     (`kdf_params`, `account_public_key`, `wrapped_private_key`,
//!     `wrapped_data_key`).
//! (b) An unauthenticated `GET /account/keys` is rejected with 401.
//! (c) An account whose bundle was never provisioned yields 409 Conflict (so the
//!     client never has to deserialize null fields).
//! (d) Zero-knowledge two-device round-trip: "device A" creates the key
//!     hierarchy and uploads only ciphertext + public key at signup; "device B"
//!     logs in, fetches the bundle, and recovers the SAME library data key with
//!     `(password + Secret Key)` — while the server DB holds no plaintext key.

mod harness;

use harness::TestServer;
use sha2::{Digest, Sha256};
use srp::ClientG2048;
use toku_core::{AccountKdfParams, AccountKeys, WrappedAccountPrivateKey, WrappedDataKey};
use toku_sync::db::SyncDatabase;

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

// ── SRP helpers (mirrors the server's email-as-identity SRP-6a) ───────────────

/// Deterministic 16-byte salt derived from the email so the test client and
/// server agree. Real clients use CSPRNG output.
fn salt_for(email: &str) -> Vec<u8> {
    let digest = Sha256::digest(format!("salt:{email}").as_bytes());
    digest[..16].to_vec()
}

fn verifier_hex(email: &str, password: &str, salt: &[u8]) -> String {
    let client = ClientG2048::<Sha256>::new();
    let v = client.compute_verifier(email.as_bytes(), password.as_bytes(), salt);
    hex::encode(v)
}

/// Sign up an account, optionally attaching the key-hierarchy bundle fields.
fn signup_with_bundle(
    server: &TestServer,
    email: &str,
    password: &str,
    bundle: Option<&BundleStrings>,
) -> (u16, serde_json::Value) {
    let salt = salt_for(email);
    let mut body = serde_json::json!({
        "email": email,
        "srp_salt": hex::encode(&salt),
        "srp_verifier": verifier_hex(email, password, &salt),
    });
    if let Some(b) = bundle {
        body["kdf_params"] = serde_json::Value::String(b.kdf_params.clone());
        body["account_public_key"] = serde_json::Value::String(b.account_public_key.clone());
        body["wrapped_private_key"] = serde_json::Value::String(b.wrapped_private_key.clone());
        body["wrapped_data_key"] = serde_json::Value::String(b.wrapped_data_key.clone());
    }
    let url = format!("{}/api/v1/account/signup", server.base_url());
    post_json(&url, &body)
}

/// Run a full user SRP login (challenge + verify). Returns the session token.
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
    assert_eq!(status, 200, "challenge should succeed: {challenge}");
    let challenge_id = challenge["challenge_id"].as_str().expect("challenge_id");
    let server_b =
        hex::decode(challenge["server_public_b"].as_str().expect("b")).expect("decode b");
    let salt = hex::decode(challenge["srp_salt"].as_str().expect("salt")).expect("decode salt");

    let verifier = client
        .process_reply(&a, email.as_bytes(), password.as_bytes(), &salt, &server_b)
        .expect("process_reply");
    let m1 = hex::encode(verifier.proof());

    let (status, body) = post_json(
        &format!("{}/api/v1/account/verify", server.base_url()),
        &serde_json::json!({ "challenge_id": challenge_id, "client_proof_m1": m1 }),
    );
    assert_eq!(status, 200, "verify should succeed: {body}");
    let m2 = hex::decode(body["server_proof_m2"].as_str().expect("m2")).expect("decode m2");
    verifier
        .verify_server(&m2)
        .expect("server proof M2 must verify");

    body["session_token"]
        .as_str()
        .expect("session_token")
        .to_string()
}

// ── Key-bundle helpers ───────────────────────────────────────────────────────

/// The four opaque strings a client uploads at signup, derived from a real
/// [`AccountKeys`] hierarchy exactly the way #123's client does it.
struct BundleStrings {
    kdf_params: String,
    account_public_key: String,
    wrapped_private_key: String,
    wrapped_data_key: String,
}

impl BundleStrings {
    fn from_account_keys(keys: &AccountKeys) -> Self {
        Self {
            kdf_params: serde_json::to_string(&keys.kdf).expect("serialize kdf"),
            // `public_key` is already a base64 X25519 string, sent verbatim.
            account_public_key: keys.public_key.clone(),
            wrapped_private_key: serde_json::to_string(&keys.wrapped_private_key)
                .expect("serialize wrapped private key"),
            wrapped_data_key: serde_json::to_string(&keys.wrapped_data_key)
                .expect("serialize wrapped data key"),
        }
    }
}

/// Reconstruct an [`AccountKeys`] from the four strings the endpoint returns,
/// mirroring how a new device rebuilds the hierarchy before unlocking. The
/// schema version is the only field not on the wire; it is pinned to v1 (the
/// sole supported version, asserted by `AccountKeys::validate`).
fn account_keys_from_response(body: &serde_json::Value) -> AccountKeys {
    let kdf: AccountKdfParams =
        serde_json::from_str(body["kdf_params"].as_str().expect("kdf_params")).expect("parse kdf");
    let wrapped_private_key: WrappedAccountPrivateKey = serde_json::from_str(
        body["wrapped_private_key"]
            .as_str()
            .expect("wrapped_private_key"),
    )
    .expect("parse wrapped private key");
    let wrapped_data_key: WrappedDataKey =
        serde_json::from_str(body["wrapped_data_key"].as_str().expect("wrapped_data_key"))
            .expect("parse wrapped data key");
    AccountKeys {
        version: 1,
        kdf,
        public_key: body["account_public_key"]
            .as_str()
            .expect("account_public_key")
            .to_string(),
        wrapped_private_key,
        wrapped_data_key,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// (a) Signup persists the bundle; `GET /account/keys` returns the exact four
/// uploaded strings under the contract's field names.
#[test]
fn account_keys_returns_exact_uploaded_bundle() {
    let server = TestServer::start();

    let secret_key = [9u8; 32];
    let password = "correct horse battery staple";
    let (keys, _data_key) = AccountKeys::create(password, &secret_key).expect("create keys");
    let bundle = BundleStrings::from_account_keys(&keys);

    let (status, body) = signup_with_bundle(&server, "a@example.com", password, Some(&bundle));
    assert_eq!(status, 200, "signup with bundle should succeed: {body}");

    let token = login_token(&server, "a@example.com", password);
    let (status, body) = get_json(
        &format!("{}/api/v1/account/keys", server.base_url()),
        Some(&token),
    );
    assert_eq!(status, 200, "GET /account/keys should succeed: {body}");

    assert_eq!(body["kdf_params"], serde_json::json!(bundle.kdf_params));
    assert_eq!(
        body["account_public_key"],
        serde_json::json!(bundle.account_public_key)
    );
    assert_eq!(
        body["wrapped_private_key"],
        serde_json::json!(bundle.wrapped_private_key)
    );
    assert_eq!(
        body["wrapped_data_key"],
        serde_json::json!(bundle.wrapped_data_key)
    );

    // Exactly the four contract fields, all non-null strings.
    let obj = body.as_object().expect("object response");
    assert_eq!(obj.len(), 4, "response must have exactly four fields");
    for key in [
        "kdf_params",
        "account_public_key",
        "wrapped_private_key",
        "wrapped_data_key",
    ] {
        assert!(obj[key].is_string(), "{key} must be a non-null string");
    }
}

/// (b) An unauthenticated request is rejected with 401.
#[test]
fn account_keys_requires_auth() {
    let server = TestServer::start();
    let (status, _) = get_json(&format!("{}/api/v1/account/keys", server.base_url()), None);
    assert_eq!(status, 401, "missing bearer must be unauthorized");

    let (status, _) = get_json(
        &format!("{}/api/v1/account/keys", server.base_url()),
        Some("not-a-real-token"),
    );
    assert_eq!(status, 401, "bogus bearer must be unauthorized");
}

/// (c) An account with no provisioned bundle yields 409 Conflict rather than
/// emitting null fields the client cannot deserialize.
#[test]
fn account_keys_unprovisioned_returns_409() {
    let server = TestServer::start();

    let password = "another good password";
    // Sign up WITHOUT any bundle fields (legacy / partial signup).
    let (status, body) = signup_with_bundle(&server, "b@example.com", password, None);
    assert_eq!(status, 200, "signup should succeed: {body}");

    let token = login_token(&server, "b@example.com", password);
    let (status, body) = get_json(
        &format!("{}/api/v1/account/keys", server.base_url()),
        Some(&token),
    );
    assert_eq!(status, 409, "unprovisioned bundle must be 409: {body}");
}

/// (d) Zero-knowledge two-device round-trip: device B recovers the SAME data key
/// device A created, using only the fetched bundle plus `(password + Secret
/// Key)`. The server DB stores no plaintext data key.
#[test]
fn second_device_recovers_same_data_key_zero_knowledge() {
    let server = TestServer::start();

    let secret_key = [42u8; 32];
    let password = "shared library passphrase";

    // Device A: create the hierarchy locally and upload only ciphertext + the
    // public key. The plaintext `data_key_a` never leaves device A.
    let (keys, data_key_a) = AccountKeys::create(password, &secret_key).expect("create keys");
    let bundle = BundleStrings::from_account_keys(&keys);
    let (status, body) = signup_with_bundle(&server, "owner@example.com", password, Some(&bundle));
    assert_eq!(status, 200, "signup with bundle should succeed: {body}");

    // Device B: a brand-new device. SRP login -> fetch bundle -> unlock.
    let token = login_token(&server, "owner@example.com", password);
    let (status, body) = get_json(
        &format!("{}/api/v1/account/keys", server.base_url()),
        Some(&token),
    );
    assert_eq!(status, 200, "GET /account/keys should succeed: {body}");

    let recovered = account_keys_from_response(&body);
    let data_key_b = recovered
        .unlock_data_key(password, &secret_key)
        .expect("device B must unlock the data key");

    assert_eq!(
        data_key_a.as_exported_bytes(),
        data_key_b.as_exported_bytes(),
        "device B must recover the exact same library data key as device A"
    );

    // Zero-knowledge: the plaintext data key must appear nowhere in the server
    // DB — only the wrapped (ciphertext) form is stored.
    let plaintext_b64 = base64_encode(data_key_a.as_exported_bytes());
    let stored_wrapped_data_key: String = {
        let db = SyncDatabase::open_no_migrate(&server.db_path()).expect("open server db");
        db.conn
            .query_row(
                "SELECT wrapped_data_key FROM users WHERE email = ?1",
                ["owner@example.com"],
                |row| row.get(0),
            )
            .expect("query wrapped_data_key")
    };
    assert!(
        !stored_wrapped_data_key.contains(&plaintext_b64),
        "the wrapped data key must not contain the plaintext data key"
    );
    assert_eq!(
        stored_wrapped_data_key, bundle.wrapped_data_key,
        "server must persist exactly the opaque ciphertext it was sent"
    );
}

/// Minimal standard-base64 encoder so the plaintext-leak check doesn't depend on
/// the exact base64 helper used inside `toku-core`.
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
