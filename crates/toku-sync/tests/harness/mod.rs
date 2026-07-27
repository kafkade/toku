//! Reusable multi-device sync test harness.
//!
//! Provides two building blocks for end-to-end sync tests:
//!
//! * [`TestServer`] — a real Toku sync relay (`toku-sync`'s Axum router) bound
//!   to a random loopback port, backed by a throwaway SQLite database. It runs
//!   on its own background Tokio runtime so the synchronous, blocking sync
//!   client can drive it over real HTTP.
//! * [`SimulatedDevice`] — a self-contained Toku client: a temp data directory
//!   with its own `toku.db`, config, and file-mode token store, registered
//!   against the server through the **real** `toku-sync-client` orchestrator.
//!   Local mutations build real [`SyncOp`]s, materialize them through the real
//!   [`MergeEngine`], and stage them for push; `push`/`pull`/`bootstrap` call
//!   the real orchestrator end-to-end.
//!
//! The harness deliberately owns op *emission* (there is no reusable
//! per-mutation emission layer in the product yet — it lives inline in CLI
//! handlers). Everything below emission — the server relay, merge engine,
//! crypto, HLC ordering, and orchestrator push/pull/apply — is the real code.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Once;

use chrono::{DateTime, Utc};
use tempfile::TempDir;
use tokio::runtime::Runtime;
use toku_core::{
    Book, EntityType, HybridClock, OpType, ProgressType, ReadingProgress, ReadingSession,
    ReadingStatus, SyncOp, TagType,
};
use toku_db::{BookRepository, Database, MergeEngine, SyncRepository};
use toku_sync::build_router;
use toku_sync::db::SyncDatabase;
use uuid::Uuid;

/// Ensure the token store never touches the OS keychain during tests.
static INIT_ENV: Once = Once::new();

/// Default passphrase used when a test does not pin its own. Encryption is
/// mandatory, so every device must enroll/login with a passphrase.
pub const DEFAULT_TEST_PASSPHRASE: &str = "harness-default-passphrase";

fn ensure_file_token_store() {
    INIT_ENV.call_once(|| {
        // SAFETY: called once, before any device spawns a thread that reads
        // the variable; the value is constant for the lifetime of the test
        // process.
        unsafe {
            std::env::set_var("TOKU_TOKEN_STORE", "file");
        }
    });
}

/// An in-process Toku sync relay server, bound to a random loopback port.
pub struct TestServer {
    base_url: String,
    // Kept alive for the lifetime of the server; dropping it shuts the
    // background server task down. Field order matters: the runtime is dropped
    // before the temp dir so in-flight tasks stop before the db file vanishes.
    _runtime: Runtime,
    _tempdir: TempDir,
}

impl TestServer {
    /// Start a fresh server with an empty, migrated database.
    pub fn start() -> Self {
        let tempdir = tempfile::tempdir().expect("create server temp dir");
        let db_path = tempdir.path().join("server.db");

        // Create + migrate the database up front, then drop the connection so
        // the per-request handlers can open their own.
        SyncDatabase::open(&db_path).expect("migrate server database");

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build server runtime");

        let listener = runtime
            .block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await })
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("read local addr");

        let router = build_router(db_path);
        runtime.spawn(async move {
            axum::serve(listener, router).await.expect("serve");
        });

        Self {
            base_url: format!("http://{addr}"),
            _runtime: runtime,
            _tempdir: tempdir,
        }
    }

    /// The server's base URL (e.g. `http://127.0.0.1:54321`).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Path to the server's SQLite database (for white-box assertions).
    pub fn db_path(&self) -> std::path::PathBuf {
        self._tempdir.path().join("server.db")
    }
}

/// A simulated Toku device with its own local database, registered against a
/// [`TestServer`] and driven through the real sync orchestrator.
pub struct SimulatedDevice {
    name: String,
    data_dir: TempDir,
    device_id: Uuid,
    clock: HybridClock,
    /// Auto-incrementing logical millisecond used when a test doesn't pin an
    /// explicit time. Starts high so explicit small values stay distinct.
    next_ms: i64,
}

impl SimulatedDevice {
    /// Register a new device against `server`, joining `library_id`.
    ///
    /// Client-side encryption is mandatory in hosted mode (issue #121), so a
    /// passphrase is always used: pass an explicit one, or `None` to fall back
    /// to [`DEFAULT_TEST_PASSPHRASE`]. Devices in the same library must share a
    /// passphrase to converge.
    pub fn register(
        server: &TestServer,
        library_id: &str,
        name: &str,
        passphrase: Option<&str>,
    ) -> Self {
        ensure_file_token_store();
        let data_dir = tempfile::tempdir().expect("create device temp dir");

        let passphrase = passphrase.or(Some(DEFAULT_TEST_PASSPHRASE));
        let outcome = toku_sync_client::init(
            data_dir.path(),
            server.base_url(),
            Some(library_id.to_string()),
            Some(name.to_string()),
            passphrase,
        )
        .expect("device init");

        let device_id = outcome.device_id.parse().expect("valid device id");
        let clock = HybridClock::new(&device_id);

        Self {
            name: name.to_string(),
            data_dir,
            device_id,
            clock,
            next_ms: 1_000,
        }
    }

    /// The library-wide unique id assigned to this device by the server.
    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The bearer token this device authenticates with (white-box: for tests
    /// that drive the server over raw HTTP).
    pub fn auth_token(&self, server: &TestServer) -> String {
        let store = toku_sync_client::TokenStore::new(self.data_dir());
        store
            .load(server.base_url())
            .expect("load auth token")
            .expect("device has an auth token")
    }

    /// The client-side encryption key derived during enrollment (white-box).
    pub fn sync_key(&self, server: &TestServer) -> toku_core::SyncKey {
        let store = toku_sync_client::TokenStore::new(self.data_dir());
        let bytes = store
            .load_sync_key(server.base_url())
            .expect("load sync key")
            .expect("device has a sync key");
        toku_core::SyncKey::from_exported_bytes(&bytes).expect("valid sync key")
    }

    fn data_dir(&self) -> &Path {
        self.data_dir.path()
    }

    /// Public path to this device's data directory (for orchestrator-level
    /// tests such as relay→account migration).
    pub fn data_dir_path(&self) -> &Path {
        self.data_dir.path()
    }

    fn open_db(&self) -> Database {
        Database::open(&self.data_dir().join("toku.db")).expect("open device db")
    }

    fn next_time(&mut self) -> DateTime<Utc> {
        let ms = self.next_ms;
        self.next_ms += 1;
        DateTime::from_timestamp_millis(ms).expect("valid timestamp")
    }

    // ── Mutations ────────────────────────────────────────────────────────────
    //
    // Each mutation builds a real SyncOp, materializes it locally through the
    // real MergeEngine (recording field provenance + HLC), and stages it for
    // the next push via SyncRepository::insert_op.

    fn apply_local(&self, op: &SyncOp) {
        let db = self.open_db();
        MergeEngine::new(&db).apply_op(op).expect("apply local op");
        SyncRepository::new(&db)
            .insert_op(op)
            .expect("stage local op");
    }

    fn emit(
        &mut self,
        entity_id: Uuid,
        op_type: OpType,
        fields: Option<serde_json::Value>,
        at: DateTime<Utc>,
    ) {
        let hlc = self.clock.now_at(at);
        let op = SyncOp::new(
            self.device_id,
            hlc,
            EntityType::Book,
            entity_id,
            op_type,
            fields,
        );
        self.apply_local(&op);
    }

    /// Create a book with the given title; returns its id. Uses an
    /// auto-incrementing logical time.
    pub fn add_book(&mut self, title: &str) -> Uuid {
        let at = self.next_time();
        self.add_book_at(title, at)
    }

    /// Create a book at an explicit physical time (for deterministic LWW).
    pub fn add_book_at(&mut self, title: &str, at: DateTime<Utc>) -> Uuid {
        let id = Uuid::now_v7();
        let fields = serde_json::json!({
            "title": title,
            "status": "want-to-read",
            "format": "physical",
        });
        self.emit(id, OpType::Create, Some(fields), at);
        id
    }

    /// Update a book's title. Uses an auto-incrementing logical time.
    pub fn set_title(&mut self, book_id: Uuid, title: &str) {
        let at = self.next_time();
        self.set_title_at(book_id, title, at);
    }

    pub fn set_title_at(&mut self, book_id: Uuid, title: &str, at: DateTime<Utc>) {
        let fields = serde_json::json!({ "title": title });
        self.emit(book_id, OpType::Update, Some(fields), at);
    }

    /// Update a book's rating (0–10). Uses an auto-incrementing logical time.
    pub fn set_rating(&mut self, book_id: Uuid, rating: i64) {
        let at = self.next_time();
        self.set_rating_at(book_id, rating, at);
    }

    pub fn set_rating_at(&mut self, book_id: Uuid, rating: i64, at: DateTime<Utc>) {
        let fields = serde_json::json!({ "rating": rating });
        self.emit(book_id, OpType::Update, Some(fields), at);
    }

    /// Delete a book. Uses an auto-incrementing logical time.
    pub fn delete_book(&mut self, book_id: Uuid) {
        let at = self.next_time();
        self.delete_book_at(book_id, at);
    }

    pub fn delete_book_at(&mut self, book_id: Uuid, at: DateTime<Utc>) {
        self.emit(book_id, OpType::Delete, None, at);
    }

    // ── Real-frontend mutations ───────────────────────────────────────────────
    //
    // These drive the product's `BookRepository` directly. Because the device
    // has a configured identity (from enrollment), every write funnels through
    // the same op-emission choke-point the CLI and FFI use — so these exercise
    // real command paths, not hand-staged ops. The op is emitted atomically
    // with the write and staged for the next push.

    /// Create a book through the real repository. Returns the new book id.
    pub fn repo_add_book(&self, title: &str) -> Uuid {
        let db = self.open_db();
        let book = Book::new(title);
        let id = book.id;
        BookRepository::new(&db)
            .create_book(&book)
            .expect("repo create_book");
        id
    }

    /// Set a book's status through the real repository.
    pub fn repo_set_status(&self, book_id: Uuid, status: ReadingStatus) {
        let db = self.open_db();
        BookRepository::new(&db)
            .update_book_status(&book_id, status)
            .expect("repo update_book_status");
    }

    /// Set a book's rating through the real repository.
    pub fn repo_set_rating(&self, book_id: Uuid, rating: i32) {
        let db = self.open_db();
        BookRepository::new(&db)
            .update_book_rating(&book_id, rating)
            .expect("repo update_book_rating");
    }

    /// Soft-delete a book through the real repository.
    pub fn repo_delete_book(&self, book_id: Uuid) {
        let db = self.open_db();
        BookRepository::new(&db)
            .delete_book(&book_id)
            .expect("repo delete_book");
    }

    /// Log a reading session through the real repository. Returns its id.
    pub fn repo_log_session(&self, book_id: Uuid) -> Uuid {
        let db = self.open_db();
        let session = ReadingSession::new(book_id);
        let id = session.id;
        BookRepository::new(&db)
            .create_reading_session(&session)
            .expect("repo create_reading_session");
        id
    }

    /// Log a page-progress entry through the real repository. Returns its id.
    pub fn repo_log_progress(&self, book_id: Uuid, value: i32) -> Uuid {
        let db = self.open_db();
        let progress = ReadingProgress::new(book_id, ProgressType::Page, value);
        let id = progress.id;
        BookRepository::new(&db)
            .log_progress(&progress)
            .expect("repo log_progress");
        id
    }

    /// Add a general tag to a book through the real repository.
    pub fn repo_add_tag(&self, book_id: Uuid, name: &str) {
        let db = self.open_db();
        BookRepository::new(&db)
            .add_typed_tag_to_book(&book_id, name, TagType::General)
            .expect("repo add_typed_tag_to_book");
    }

    // ── Sync ─────────────────────────────────────────────────────────────────

    /// Push all locally-pending ops to the server.
    pub fn push(&self) -> toku_sync_client::PushOutcome {
        toku_sync_client::push(self.data_dir()).expect("push")
    }

    /// Pull and apply remote ops from the server.
    pub fn pull(&self) -> toku_sync_client::PullOutcome {
        toku_sync_client::pull(self.data_dir()).expect("pull")
    }

    /// Bootstrap from a server snapshot (falling back to a full op-log pull
    /// when no snapshot exists).
    pub fn bootstrap(&self) -> toku_sync_client::BootstrapOutcome {
        toku_sync_client::bootstrap(self.data_dir()).expect("bootstrap")
    }

    // ── State readers (assert against materialized tables) ────────────────────

    /// True when the book exists locally and is not soft-deleted.
    pub fn book_exists(&self, book_id: Uuid) -> bool {
        let db = self.open_db();
        BookRepository::new(&db).get_book(&book_id).is_ok()
    }

    pub fn book_title(&self, book_id: Uuid) -> Option<String> {
        let db = self.open_db();
        BookRepository::new(&db)
            .get_book(&book_id)
            .ok()
            .map(|b| b.title)
    }

    pub fn book_rating(&self, book_id: Uuid) -> Option<i32> {
        let db = self.open_db();
        BookRepository::new(&db)
            .get_book(&book_id)
            .ok()
            .and_then(|b| b.rating)
    }

    pub fn book_status(&self, book_id: Uuid) -> Option<String> {
        let db = self.open_db();
        BookRepository::new(&db)
            .get_book(&book_id)
            .ok()
            .map(|b| b.status.to_string())
    }

    /// Number of reading sessions recorded locally for a book.
    pub fn session_count(&self, book_id: Uuid) -> usize {
        let db = self.open_db();
        db.conn
            .query_row(
                "SELECT COUNT(*) FROM reading_sessions WHERE book_id = ?1",
                [book_id.to_string()],
                |r| r.get::<_, i64>(0),
            )
            .expect("count sessions") as usize
    }

    /// The most recent progress value recorded locally for a book, if any.
    pub fn latest_progress(&self, book_id: Uuid) -> Option<i32> {
        let db = self.open_db();
        BookRepository::new(&db)
            .get_latest_progress(&book_id)
            .ok()
            .flatten()
            .map(|p| p.value)
    }

    /// True when the book carries a tag with the given name locally.
    pub fn has_tag(&self, book_id: Uuid, name: &str) -> bool {
        let db = self.open_db();
        BookRepository::new(&db)
            .get_book_tags(&book_id)
            .map(|tags| tags.iter().any(|t| t.name == name))
            .unwrap_or(false)
    }

    /// Number of ops staged locally that have not yet been pushed.
    pub fn pending_ops(&self) -> usize {
        let db = self.open_db();
        SyncRepository::new(&db)
            .get_unpushed_ops()
            .expect("read unpushed ops")
            .len()
    }

    /// Number of non-deleted books in the local library.
    pub fn book_count(&self) -> usize {
        let db = self.open_db();
        BookRepository::new(&db)
            .list_books()
            .expect("list books")
            .len()
    }

    /// Clear the local "pushed" marker on all staged ops, forcing the next
    /// `push` to re-send them. Used to simulate a client that delivered ops to
    /// the server but lost the success acknowledgement (crash/network drop),
    /// exercising the server's op-id dedup on retry.
    pub fn force_repush(&self) {
        let db = self.open_db();
        db.conn
            .execute("UPDATE sync_ops SET pushed_at = NULL", [])
            .expect("reset push state");
    }
}
