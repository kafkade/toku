//! First-opt-in op-backfill (#199, ADR-013 D2) + new-device bootstrap wiring (D3).
//!
//! These drive the **real** `toku-sync-client` orchestrator (`signup`/`enroll`/
//! `login`/`push`/`bootstrap`) — the logic behind the `toku sync …` subcommands —
//! against a real in-process `toku-sync` server, using the **real**
//! `BookRepository` to seed library state (no hand-staged ops). They assert the
//! two ADR-013 decisions this issue implements:
//!
//! * **D2 — first opt-in uploads existing state.** Rows created *before* opt-in
//!   have no op (a device identity is minted *by* opt-in, and #194's emission is a
//!   no-op until then). On `signup` (and a fresh-library `enroll`) the client
//!   backfills `Create` ops for every pre-existing syncable row and pushes them —
//!   so a brand-new device restores the full library **without a manual
//!   `compact`**.
//! * **D3 — new-device bootstrap is wired + exposed.** An active `enroll` runs
//!   bootstrap automatically; an approval-pending device defers it to the first
//!   post-approval `login`. Bootstrap restores from a server snapshot after
//!   compaction, falling back to an op-log pull when none exists.

mod harness;

use std::path::Path;
use std::sync::Once;

use harness::TestServer;
use tempfile::TempDir;
use toku_core::{
    Book, HybridClock, ProgressType, ReadingProgress, ReadingSession, SecretKey, SyncKey, TagType,
    encrypt_snapshot,
};
use toku_db::{BookRepository, Database, SnapshotRepository, SyncRepository};
use uuid::Uuid;

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

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
}

fn data_dir() -> TempDir {
    tempfile::tempdir().expect("create client data dir")
}

fn open_db(dir: &Path) -> Database {
    Database::open(&dir.join("toku.db")).expect("open device db")
}

// ── Seeding via the real repository ───────────────────────────────────────────
//
// Before opt-in there is no device identity, so these writes stage **zero** ops
// (offline-first: emission is a no-op without a device). After opt-in the same
// calls emit ops through the #194 choke-point.

fn seed_book(dir: &Path, title: &str) -> Uuid {
    let db = open_db(dir);
    let book = Book::new(title);
    let id = book.id;
    BookRepository::new(&db)
        .create_book(&book)
        .expect("create_book");
    id
}

fn seed_session(dir: &Path, book_id: Uuid) -> Uuid {
    let db = open_db(dir);
    let session = ReadingSession::new(book_id);
    let id = session.id;
    BookRepository::new(&db)
        .create_reading_session(&session)
        .expect("create_reading_session");
    id
}

fn seed_progress(dir: &Path, book_id: Uuid, value: i32) -> Uuid {
    let db = open_db(dir);
    let progress = ReadingProgress::new(book_id, ProgressType::Page, value);
    let id = progress.id;
    BookRepository::new(&db)
        .log_progress(&progress)
        .expect("log_progress");
    id
}

fn seed_tag(dir: &Path, book_id: Uuid, name: &str) {
    let db = open_db(dir);
    BookRepository::new(&db)
        .add_typed_tag_to_book(&book_id, name, TagType::General)
        .expect("add_typed_tag_to_book");
}

// ── Local-state readers ───────────────────────────────────────────────────────

fn pending_ops(dir: &Path) -> usize {
    let db = open_db(dir);
    SyncRepository::new(&db)
        .get_unpushed_ops()
        .expect("read unpushed ops")
        .len()
}

fn book_count(dir: &Path) -> usize {
    let db = open_db(dir);
    BookRepository::new(&db)
        .list_books()
        .expect("list books")
        .len()
}

fn book_titles(dir: &Path) -> Vec<String> {
    let db = open_db(dir);
    let mut titles: Vec<String> = BookRepository::new(&db)
        .list_books()
        .expect("list books")
        .into_iter()
        .map(|b| b.title)
        .collect();
    titles.sort();
    titles
}

fn session_count(dir: &Path, book_id: Uuid) -> i64 {
    let db = open_db(dir);
    db.conn
        .query_row(
            "SELECT COUNT(*) FROM reading_sessions WHERE book_id = ?1",
            [book_id.to_string()],
            |r| r.get(0),
        )
        .expect("count sessions")
}

fn latest_progress(dir: &Path, book_id: Uuid) -> Option<i32> {
    let db = open_db(dir);
    BookRepository::new(&db)
        .get_latest_progress(&book_id)
        .expect("get_latest_progress")
        .map(|p| p.value)
}

fn has_tag(dir: &Path, book_id: Uuid, name: &str) -> bool {
    let db = open_db(dir);
    BookRepository::new(&db)
        .get_book_tags(&book_id)
        .map(|tags| tags.iter().any(|t| t.name == name))
        .unwrap_or(false)
}

/// Reproduce `toku sync compact`: export a snapshot, encrypt it client-side, and
/// upload it (which prunes the pre-snapshot op history on the server). Kept as a
/// manual maintenance step (ADR-013) so the after-compaction restore path stays
/// exercised end-to-end.
fn compact(dir: &Path, server: &TestServer) {
    let db = open_db(dir);
    let sync_repo = SyncRepository::new(&db);
    let snapshot_repo = SnapshotRepository::new(&db);
    let device = sync_repo
        .get_device()
        .expect("get_device")
        .expect("device identity");
    let hlc = HybridClock::new(&device.device_id).now().to_canonical();

    let store = toku_sync_client::TokenStore::new(dir);
    let token = store
        .load(server.base_url())
        .expect("load token")
        .expect("device token");
    let key_bytes = store
        .load_sync_key(server.base_url())
        .expect("load sync key")
        .expect("sync key");
    let key = SyncKey::from_exported_bytes(&key_bytes).expect("valid sync key");

    let snapshot = snapshot_repo
        .export_snapshot(device.device_id, &hlc)
        .expect("export snapshot");
    let json = serde_json::to_string(&snapshot).expect("serialize snapshot");
    let envelope = encrypt_snapshot(&key, &json).expect("encrypt snapshot");
    let blob = serde_json::to_string(&envelope).expect("serialize envelope");

    let client = toku_sync_client::SyncClient::new(server.base_url()).expect("client");
    rt().block_on(client.upload_snapshot(&token, &blob, &hlc))
        .expect("upload snapshot");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// AC#1: a fresh library that opts in via `signup` uploads its pre-existing state
/// automatically — a brand-new device restores everything WITHOUT a manual
/// `compact`.
#[test]
fn fresh_signup_backfills_existing_library_without_compact() {
    use_file_token_store();
    let server = TestServer::start();
    let dir_a = data_dir();
    let secret = SecretKey::generate().expect("generate secret key");

    // Seed BEFORE opt-in: no device identity yet, so these stage zero ops.
    let dune = seed_book(dir_a.path(), "Dune");
    let earthsea = seed_book(dir_a.path(), "A Wizard of Earthsea");
    seed_session(dir_a.path(), dune);
    seed_progress(dir_a.path(), dune, 42);
    seed_tag(dir_a.path(), earthsea, "fantasy");
    assert_eq!(
        pending_ops(dir_a.path()),
        0,
        "pre-opt-in writes must stage no ops (offline-first no-op)"
    );

    let signup = toku_sync_client::signup(
        dir_a.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    // D2: one backfilled + pushed op per pre-existing syncable row.
    let bf = &signup.backfill;
    assert_eq!(bf.books, 2, "both books backfilled");
    assert_eq!(bf.sessions, 1, "the session backfilled");
    assert_eq!(bf.progress, 1, "the progress entry backfilled");
    assert_eq!(bf.tags, 1, "the tag backfilled");
    assert_eq!(bf.ops_total, 5);
    assert_eq!(bf.pushed, 5, "all backfilled ops accepted by the server");

    // Prove the state reached the server WITHOUT a manual compact: a brand-new
    // device restores the full library through the orchestrator's bootstrap.
    let dir_b = data_dir();
    let enroll = toku_sync_client::enroll(
        dir_b.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("phone".to_string()),
        Some(signup.library_id.clone()),
    )
    .expect("enroll should succeed");

    assert!(
        enroll.bootstrap.is_some(),
        "an active enroll must auto-bootstrap (D3)"
    );
    assert!(
        enroll.backfill.is_none(),
        "joining an existing library must not backfill"
    );
    assert_eq!(book_count(dir_b.path()), 2, "both books restored");
    assert_eq!(
        book_titles(dir_b.path()),
        vec!["A Wizard of Earthsea".to_string(), "Dune".to_string()]
    );
    assert_eq!(session_count(dir_b.path(), dune), 1, "session restored");
    assert_eq!(
        latest_progress(dir_b.path(), dune),
        Some(42),
        "progress restored"
    );
    assert!(has_tag(dir_b.path(), earthsea, "fantasy"), "tag restored");
}

/// AC#2: a new device restores full prior state via the auto-wired bootstrap.
/// Here the data is written *after* opt-in (normal ongoing ops) and pushed; the
/// enrolling device converges via the bootstrap op-log pull (no snapshot yet).
#[test]
fn new_device_enroll_restores_full_prior_state() {
    use_file_token_store();
    let server = TestServer::start();
    let dir_a = data_dir();
    let secret = SecretKey::generate().expect("generate secret key");

    let signup = toku_sync_client::signup(
        dir_a.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");
    assert_eq!(
        signup.backfill.ops_total, 0,
        "fresh library has nothing to backfill"
    );

    // Ongoing mutations after opt-in emit ops through the normal path.
    let book = seed_book(dir_a.path(), "The Left Hand of Darkness");
    seed_session(dir_a.path(), book);
    seed_progress(dir_a.path(), book, 100);
    seed_tag(dir_a.path(), book, "sci-fi");
    let pushed = toku_sync_client::push(dir_a.path()).expect("push");
    assert!(pushed.accepted >= 4, "ongoing ops pushed: {pushed:?}");

    let dir_b = data_dir();
    let enroll = toku_sync_client::enroll(
        dir_b.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("phone".to_string()),
        Some(signup.library_id.clone()),
    )
    .expect("enroll should succeed");

    let bootstrap = enroll.bootstrap.expect("active enroll auto-bootstraps");
    assert!(
        !bootstrap.snapshot_applied,
        "no compaction ran, so bootstrap falls back to an op-log pull"
    );
    assert_eq!(book_count(dir_b.path()), 1);
    assert_eq!(
        book_titles(dir_b.path()),
        vec!["The Left Hand of Darkness".to_string()]
    );
    assert_eq!(session_count(dir_b.path(), book), 1);
    assert_eq!(latest_progress(dir_b.path(), book), Some(100));
    assert!(has_tag(dir_b.path(), book, "sci-fi"));
}

/// AC#3: enrollment after compaction restores the full library from the server
/// snapshot (the op history has been pruned), then converges via the tail pull.
#[test]
fn enroll_after_compaction_restores_via_snapshot() {
    use_file_token_store();
    let server = TestServer::start();
    let dir_a = data_dir();
    let secret = SecretKey::generate().expect("generate secret key");

    let signup = toku_sync_client::signup(
        dir_a.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    let dune = seed_book(dir_a.path(), "Dune");
    let hyperion = seed_book(dir_a.path(), "Hyperion");
    seed_session(dir_a.path(), dune);
    seed_progress(dir_a.path(), dune, 55);
    seed_tag(dir_a.path(), hyperion, "sci-fi");
    toku_sync_client::push(dir_a.path()).expect("push");

    // Compact: upload an encrypted snapshot and prune the op history server-side.
    compact(dir_a.path(), &server);

    let dir_b = data_dir();
    let enroll = toku_sync_client::enroll(
        dir_b.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("phone".to_string()),
        Some(signup.library_id.clone()),
    )
    .expect("enroll should succeed");

    let bootstrap = enroll.bootstrap.expect("active enroll auto-bootstraps");
    assert!(
        bootstrap.snapshot_applied,
        "after compaction the server serves a snapshot"
    );
    assert_eq!(bootstrap.snapshot_books, 2, "snapshot carried both books");
    assert_eq!(
        book_count(dir_b.path()),
        2,
        "full library restored from snapshot"
    );
    assert_eq!(session_count(dir_b.path(), dune), 1);
    assert_eq!(latest_progress(dir_b.path(), dune), Some(55));
    assert!(has_tag(dir_b.path(), hyperion, "sci-fi"));
}

/// Backfill is idempotent: re-running it after opt-in synthesizes no new ops
/// (every pre-existing row already carries a Create op).
#[test]
fn backfill_is_idempotent() {
    use_file_token_store();
    let server = TestServer::start();
    let dir = data_dir();
    let secret = SecretKey::generate().expect("generate secret key");

    seed_book(dir.path(), "Dune");
    let signup = toku_sync_client::signup(
        dir.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");
    assert_eq!(signup.backfill.books, 1);

    // A second backfill pass must find nothing new.
    let db = open_db(dir.path());
    let again = toku_db::backfill_sync_ops(&db).expect("re-run backfill");
    assert_eq!(
        again.total(),
        0,
        "re-running backfill must not duplicate ops"
    );
}

/// D3 defer: an approval-pending device does not bootstrap at enroll time; the
/// deferred bootstrap runs on the first post-approval `login`.
#[test]
fn pending_device_defers_bootstrap_until_post_approval_login() {
    use_file_token_store();
    let server = TestServer::start();
    let dir_a = data_dir();
    let secret = SecretKey::generate().expect("generate secret key");

    let signup = toku_sync_client::signup(
        dir_a.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("laptop".to_string()),
    )
    .expect("signup should succeed");

    // Seed + push some state to restore later.
    let book = seed_book(dir_a.path(), "Neuromancer");
    seed_progress(dir_a.path(), book, 77);
    toku_sync_client::push(dir_a.path()).expect("push");

    // Enable the device-approval gate (admin, via the owner's user session).
    let admin_token = toku_sync_client::TokenStore::new(dir_a.path())
        .load_user_session(server.base_url())
        .expect("load user session")
        .expect("owner user session");
    let (status, _) = put_auth(
        &format!("{}/api/v1/admin/device-approvals", server.base_url()),
        &admin_token,
        &serde_json::json!({ "required": true }),
    );
    assert_eq!(status, 200, "enabling device approvals should succeed");

    // Enroll a second device into the existing library: held pending, no session.
    let dir_b = data_dir();
    let enroll = toku_sync_client::enroll(
        dir_b.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
        Some("phone".to_string()),
        Some(signup.library_id.clone()),
    )
    .expect("enroll should succeed");
    assert_eq!(
        enroll.device_status, "pending",
        "second device is held pending"
    );
    assert!(
        enroll.bootstrap.is_none(),
        "a pending device must defer bootstrap (no session yet)"
    );
    assert_eq!(book_count(dir_b.path()), 0, "nothing restored yet");

    // Owner approves the pending device.
    let (status, _) = post_auth(
        &format!(
            "{}/api/v1/devices/{}/approval",
            server.base_url(),
            enroll.device_id
        ),
        &admin_token,
        &serde_json::json!({ "decision": "approve" }),
    );
    assert_eq!(status, 200, "approval should succeed");

    // First post-approval login mints a session and runs the deferred bootstrap.
    let login = toku_sync_client::login(
        dir_b.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
    )
    .expect("login should succeed");
    assert!(
        login.bootstrap.is_some(),
        "the first post-approval login runs the deferred bootstrap (D3)"
    );
    assert_eq!(
        book_count(dir_b.path()),
        1,
        "state restored after approval+login"
    );
    assert_eq!(latest_progress(dir_b.path(), book), Some(77));

    // A subsequent login must NOT re-bootstrap (gated by the local marker).
    let login2 = toku_sync_client::login(
        dir_b.path(),
        server.base_url(),
        "owner@example.com",
        "owner-password",
        &secret,
    )
    .expect("second login should succeed");
    assert!(
        login2.bootstrap.is_none(),
        "an already-bootstrapped device must not re-run bootstrap on routine login"
    );
}

// ── Minimal authenticated HTTP helpers (admin approval endpoints) ─────────────

fn put_auth(url: &str, bearer: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
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

fn post_auth(url: &str, bearer: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
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
