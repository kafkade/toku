//! Integration tests for the account-based CLI auth flows (issue #123).
//!
//! These drive the **real** `toku-sync-client` orchestrator (`signup`/`login`/
//! `enroll`) — the logic behind `toku sync signup|login|enroll` — against a real
//! in-process `toku-sync` server. They assert the zero-knowledge, Secret-Key
//! gated UX:
//!
//! * `signup` creates an account, generates the key hierarchy, and enrolls the
//!   first device (admin) in one shot.
//! * The Secret Key is never written to disk (config or token store).
//! * A wrong password is rejected client-side with no server-side disclosure.
//! * Subsequent signups are gated by the open-registration toggle.
//! * New-device `enroll` recovers the shared library data key the zero-knowledge
//!   way via the #143 `GET /api/v1/account/keys` endpoint: a second device
//!   SRP-logs-in, fetches the wrapped key bundle, and unwraps the *same* SyncKey
//!   as the signup device (byte-for-byte), without the server ever seeing the
//!   password or Secret Key.

mod harness;

use std::path::Path;
use std::sync::Once;

use harness::TestServer;
use tempfile::TempDir;
use toku_core::SecretKey;

static INIT_ENV: Once = Once::new();

/// Force the file-backed token store so tests never touch the OS keychain.
fn use_file_token_store() {
    INIT_ENV.call_once(|| {
        // SAFETY: set once, before any orchestrator call spawns threads.
        unsafe {
            std::env::set_var("TOKU_TOKEN_STORE", "file");
        }
    });
}

fn data_dir() -> TempDir {
    tempfile::tempdir().expect("create client data dir")
}

/// Recursively collect the contents of every file under `dir`.
fn all_file_contents(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(contents) = std::fs::read_to_string(&p) {
                out.push(contents);
            }
        }
    }
    out
}

#[test]
fn signup_creates_account_and_enrolls_first_device() {
    use_file_token_store();
    let server = TestServer::start();
    let dir = data_dir();
    let secret_key = SecretKey::generate().expect("generate secret key");

    let outcome = toku_sync_client::signup(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "correct horse battery staple",
        &secret_key,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    assert_eq!(outcome.email, "owner@example.com");
    // First account on a fresh instance becomes the admin.
    assert_eq!(outcome.role, "admin");
    assert_eq!(outcome.device_name, "laptop");
    assert_eq!(outcome.device_status, "active");
    assert!(!outcome.device_id.is_empty());
    assert!(!outcome.library_id.is_empty());
    // The Secret Key is surfaced once for the Emergency Kit.
    assert_eq!(outcome.secret_key, secret_key.format());

    // A device-session token and sync key must be persisted for push/pull, and a
    // user-session token for account-scoped calls.
    let store = toku_sync_client::TokenStore::new(dir.path());
    assert!(
        store.load(server.base_url()).expect("load token").is_some(),
        "device session token should be stored"
    );
    assert!(
        store
            .load_sync_key(server.base_url())
            .expect("load sync key")
            .is_some(),
        "sync key should be stored"
    );
    assert!(
        store
            .load_user_session(server.base_url())
            .expect("load user session")
            .is_some(),
        "user session token should be stored"
    );
}

#[test]
fn secret_key_is_never_persisted_to_disk() {
    use_file_token_store();
    let server = TestServer::start();
    let dir = data_dir();
    let secret_key = SecretKey::generate().expect("generate secret key");
    let formatted = secret_key.format();

    toku_sync_client::signup(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "a-strong-password",
        &secret_key,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    // The formatted Secret Key must not appear anywhere under the data dir
    // (config, token store, or any other artifact).
    for contents in all_file_contents(dir.path()) {
        assert!(
            !contents.contains(&formatted),
            "Secret Key must never be written to disk"
        );
    }
}

#[test]
fn login_with_wrong_password_is_rejected() {
    use_file_token_store();
    let server = TestServer::start();
    let dir = data_dir();
    let secret_key = SecretKey::generate().expect("generate secret key");

    toku_sync_client::signup(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "the-real-password",
        &secret_key,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    let err = toku_sync_client::login(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "WRONG-password",
        &secret_key,
    )
    .expect_err("login with wrong password must fail");

    // SRP rejects the proof; the message must not disclose which secret was
    // wrong beyond a generic credential error.
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("incorrect email or password") || msg.contains("unauthorized"),
        "unexpected error: {msg}"
    );
}

#[test]
fn login_succeeds_with_correct_credentials() {
    use_file_token_store();
    let server = TestServer::start();
    let dir = data_dir();
    let secret_key = SecretKey::generate().expect("generate secret key");

    toku_sync_client::signup(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "the-real-password",
        &secret_key,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    let outcome = toku_sync_client::login(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "the-real-password",
        &secret_key,
    )
    .expect("login with correct credentials should succeed");

    assert_eq!(outcome.email, "owner@example.com");
    assert_eq!(outcome.role, "admin");
    // With the #143 `GET /api/v1/account/keys` endpoint live, login fetches the
    // wrapped key bundle and unwraps the leaf data key during the SRP session.
    assert!(
        outcome.data_key_unlocked,
        "login should unlock the data key via the #143 account-keys endpoint"
    );
}

#[test]
fn second_signup_is_blocked_when_registration_is_closed() {
    use_file_token_store();
    let server = TestServer::start();
    let admin_dir = data_dir();
    let admin_key = SecretKey::generate().expect("generate secret key");

    toku_sync_client::signup(
        admin_dir.path(),
        server.base_url(),
        "admin@example.com",
        "admin-password",
        &admin_key,
        Some("admin-laptop".to_string()),
    )
    .expect("first signup should succeed");

    // Registration defaults to closed after the first (admin) account exists.
    let second_dir = data_dir();
    let second_key = SecretKey::generate().expect("generate secret key");
    let err = toku_sync_client::signup(
        second_dir.path(),
        server.base_url(),
        "intruder@example.com",
        "another-password",
        &second_key,
        Some("intruder-laptop".to_string()),
    )
    .expect_err("second signup must be blocked when registration is closed");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("registration is closed") || msg.contains("forbidden"),
        "unexpected error: {msg}"
    );
}

#[test]
fn enroll_new_device_recovers_shared_data_key() {
    // New-device enrollment recovers the shared library data key the
    // zero-knowledge way via `GET /api/v1/account/keys` (issue #143): the new
    // device SRP-logs-in, fetches the wrapped key bundle, and unwraps the *same*
    // SyncKey as the signup device — the server only ever stores ciphertext.
    use_file_token_store();
    let server = TestServer::start();
    let owner_dir = data_dir();
    let secret_key = SecretKey::generate().expect("generate secret key");

    let signup = toku_sync_client::signup(
        owner_dir.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret_key,
        Some("first-device".to_string()),
    )
    .expect("signup should succeed");

    // The SyncKey the first device derived and stored locally.
    let owner_store = toku_sync_client::TokenStore::new(owner_dir.path());
    let owner_sync_key = owner_store
        .load_sync_key(server.base_url())
        .expect("load owner sync key")
        .expect("owner sync key should be stored");

    // Enroll a brand-new device into the SAME library (pass the signup device's
    // library_id; omitting it would mint a fresh library by design — the account
    // data key is shared either way).
    let new_dir = data_dir();
    let outcome = toku_sync_client::enroll(
        new_dir.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret_key,
        Some("second-device".to_string()),
        Some(signup.library_id.clone()),
    )
    .expect("enroll should recover the shared data key via the account-keys endpoint");

    assert_eq!(outcome.email, "owner@example.com");
    assert_eq!(outcome.device_name, "second-device");
    assert_eq!(
        outcome.library_id, signup.library_id,
        "the new device must join the existing library"
    );
    // The owner's own second device is active by default (no approval toggle).
    assert_eq!(outcome.device_status, "active");
    assert!(!outcome.device_id.is_empty());
    assert_ne!(
        outcome.device_id, signup.device_id,
        "the new device must get its own id"
    );

    // The crux: the recovered SyncKey is byte-for-byte identical to the signup
    // device's — true zero-knowledge multi-device key recovery.
    let new_store = toku_sync_client::TokenStore::new(new_dir.path());
    let new_sync_key = new_store
        .load_sync_key(server.base_url())
        .expect("load new device sync key")
        .expect("new device sync key should be stored");
    assert_eq!(
        new_sync_key, owner_sync_key,
        "the enrolled device must recover the SAME SyncKey, byte for byte"
    );
}
