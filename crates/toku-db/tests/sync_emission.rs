//! Integration tests for the sync-op emission choke-point in `BookRepository`.
//!
//! Every create/update/delete on a syncable entity (Book, Session, Progress,
//! Tag) must stage exactly one `SyncOp`, atomically with the write, and must be
//! a complete no-op when no device identity is configured (offline-first).

use toku_core::{
    Book, EntityType, OpType, PaceRating, ProgressType, ReadingProgress, ReadingSession,
    ReadingStatus, SyncOp, TagType,
};
use toku_db::{BookRepository, Database, SyncRepository};

fn db_with_device() -> Database {
    let db = Database::open_in_memory().unwrap();
    SyncRepository::new(&db)
        .get_or_create_device("test-device")
        .unwrap();
    db
}

fn ops(db: &Database) -> Vec<SyncOp> {
    SyncRepository::new(db).get_unpushed_ops().unwrap()
}

fn ops_for(db: &Database, entity_type: EntityType) -> Vec<SyncOp> {
    ops(db)
        .into_iter()
        .filter(|o| o.entity_type == entity_type)
        .collect()
}

fn make_book(title: &str) -> Book {
    Book::new(title)
}

// ── Book ────────────────────────────────────────────────────────────────

#[test]
fn create_book_emits_one_book_create_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let mut book = make_book("Dune");
    book.rating = Some(8);
    book.page_count = Some(412);
    repo.create_book(&book).unwrap();

    let book_ops = ops_for(&db, EntityType::Book);
    assert_eq!(book_ops.len(), 1, "expected exactly one Book op");
    let op = &book_ops[0];
    assert_eq!(op.op_type, OpType::Create);
    assert_eq!(op.entity_id, book.id);

    let fields = op.fields.as_ref().expect("create op must carry fields");
    assert_eq!(fields["title"], "Dune");
    assert_eq!(fields["rating"], 8);
    assert_eq!(fields["page_count"], 412);
    assert_eq!(fields["status"], "want-to-read");
    // work_id is intentionally omitted (Work rows are not synced yet).
    assert!(fields.get("work_id").is_none());
}

#[test]
fn update_status_emits_one_update_op_and_records_provenance() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    assert!(
        repo.update_book_status(&book.id, ReadingStatus::Reading)
            .unwrap()
    );

    let update_ops: Vec<_> = ops_for(&db, EntityType::Book)
        .into_iter()
        .filter(|o| o.op_type == OpType::Update)
        .collect();
    assert_eq!(update_ops.len(), 1);
    assert_eq!(update_ops[0].fields.as_ref().unwrap()["status"], "reading");

    // Provenance for the status field must be recorded so a staler remote edit
    // can't clobber this local write on the next pull.
    let sync_hlc: Option<String> = db
        .conn
        .query_row(
            "SELECT sync_hlc FROM metadata_provenance WHERE book_id = ?1 AND field_name = 'status'",
            [book.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sync_hlc.is_some(), "status provenance sync_hlc must be set");
}

#[test]
fn update_rating_emits_one_update_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    assert!(repo.update_book_rating(&book.id, 10).unwrap());

    let update_ops: Vec<_> = ops_for(&db, EntityType::Book)
        .into_iter()
        .filter(|o| o.op_type == OpType::Update)
        .collect();
    assert_eq!(update_ops.len(), 1);
    assert_eq!(update_ops[0].fields.as_ref().unwrap()["rating"], 10);
}

#[test]
fn delete_book_emits_one_delete_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    assert!(repo.delete_book(&book.id).unwrap());

    let delete_ops: Vec<_> = ops_for(&db, EntityType::Book)
        .into_iter()
        .filter(|o| o.op_type == OpType::Delete)
        .collect();
    assert_eq!(delete_ops.len(), 1);
    assert_eq!(delete_ops[0].entity_id, book.id);
    assert!(delete_ops[0].fields.is_none());
}

#[test]
fn delete_missing_book_emits_no_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let missing = uuid::Uuid::now_v7();
    assert!(!repo.delete_book(&missing).unwrap());
    assert_eq!(ops(&db).len(), 0);
}

// ── Offline-first guard ──────────────────────────────────────────────────

#[test]
fn no_device_means_no_ops_but_write_succeeds() {
    // No device identity configured — op creation must be a complete no-op,
    // while the underlying write still happens.
    let db = Database::open_in_memory().unwrap();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();
    repo.update_book_status(&book.id, ReadingStatus::Reading)
        .unwrap();
    repo.update_book_rating(&book.id, 7).unwrap();
    repo.delete_book(&book.id).unwrap();

    assert_eq!(ops(&db).len(), 0, "offline mode must not stage any ops");
    // The write path itself must not be blocked by the absence of sync.
    assert!(repo.get_book_including_deleted(&book.id).is_ok());
}

// ── Atomicity ────────────────────────────────────────────────────────────

#[test]
fn failed_op_rolls_back_the_write() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    // Force any op insert to fail; the enclosing transaction must roll back the
    // book write too, so neither the row nor the op survives.
    db.conn
        .execute_batch(
            "CREATE TRIGGER fail_ops BEFORE INSERT ON sync_ops
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();

    let book = make_book("Dune");
    let result = repo.create_book(&book);
    assert!(result.is_err(), "create_book should surface the op failure");

    // Drop the trigger so the assertion queries work.
    db.conn.execute_batch("DROP TRIGGER fail_ops").unwrap();

    assert!(
        repo.get_book(&book.id).is_err(),
        "book write must be rolled back with the failed op"
    );
    assert_eq!(ops(&db).len(), 0, "no op should have been committed");
}

// ── Session / Progress ───────────────────────────────────────────────────

#[test]
fn create_session_emits_session_create_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    let mut session = ReadingSession::new(book.id);
    session.start_page = Some(1);
    session.end_page = Some(50);
    repo.create_reading_session(&session).unwrap();

    let session_ops = ops_for(&db, EntityType::Session);
    assert_eq!(session_ops.len(), 1);
    let op = &session_ops[0];
    assert_eq!(op.op_type, OpType::Create);
    assert_eq!(op.entity_id, session.id);
    let fields = op.fields.as_ref().unwrap();
    assert_eq!(fields["book_id"], book.id.to_string());
    assert_eq!(fields["start_page"], 1);
    assert_eq!(fields["end_page"], 50);
}

#[test]
fn log_progress_emits_progress_create_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    let progress = ReadingProgress::new(book.id, ProgressType::Page, 120);
    repo.log_progress(&progress).unwrap();

    let progress_ops = ops_for(&db, EntityType::Progress);
    assert_eq!(progress_ops.len(), 1);
    let op = &progress_ops[0];
    assert_eq!(op.op_type, OpType::Create);
    assert_eq!(op.entity_id, progress.id);
    let fields = op.fields.as_ref().unwrap();
    assert_eq!(fields["book_id"], book.id.to_string());
    assert_eq!(fields["progress_type"], "page");
    assert_eq!(fields["value"], 120);
}

// ── Tags ─────────────────────────────────────────────────────────────────

#[test]
fn add_and_remove_tag_emit_create_then_delete_ops() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    repo.add_typed_tag_to_book(&book.id, "sci-fi", TagType::General)
        .unwrap();

    let tag_ops = ops_for(&db, EntityType::Tag);
    assert_eq!(tag_ops.len(), 1);
    assert_eq!(tag_ops[0].op_type, OpType::Create);
    // Tag ops are book-scoped associations: entity_id is the book id.
    assert_eq!(tag_ops[0].entity_id, book.id);
    assert_eq!(tag_ops[0].fields.as_ref().unwrap()["tag_name"], "sci-fi");
    assert_eq!(tag_ops[0].fields.as_ref().unwrap()["tag_type"], "general");

    repo.remove_typed_tag_from_book(&book.id, "sci-fi", TagType::General)
        .unwrap();

    let tag_ops = ops_for(&db, EntityType::Tag);
    assert_eq!(tag_ops.len(), 2);
    assert_eq!(tag_ops[1].op_type, OpType::Delete);
    assert_eq!(tag_ops[1].fields.as_ref().unwrap()["tag_name"], "sci-fi");
}

#[test]
fn adding_same_tag_twice_emits_only_one_op() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    repo.add_typed_tag_to_book(&book.id, "sci-fi", TagType::General)
        .unwrap();
    // Second add is a no-op association (INSERT OR IGNORE) → no extra op.
    repo.add_typed_tag_to_book(&book.id, "sci-fi", TagType::General)
        .unwrap();

    assert_eq!(ops_for(&db, EntityType::Tag).len(), 1);
}

#[test]
fn set_pace_replaces_tag_with_delete_then_create() {
    let db = db_with_device();
    let repo = BookRepository::new(&db);

    let book = make_book("Dune");
    repo.create_book(&book).unwrap();

    repo.set_book_pace(&book.id, PaceRating::Fast).unwrap();
    // First set: no existing pace tag, so only a Create.
    let tag_ops = ops_for(&db, EntityType::Tag);
    assert_eq!(tag_ops.len(), 1);
    assert_eq!(tag_ops[0].op_type, OpType::Create);
    assert_eq!(tag_ops[0].fields.as_ref().unwrap()["tag_name"], "fast");

    repo.set_book_pace(&book.id, PaceRating::Slow).unwrap();
    // Second set: remove old "fast" (Delete) + add "slow" (Create).
    let tag_ops = ops_for(&db, EntityType::Tag);
    assert_eq!(tag_ops.len(), 3);
    assert_eq!(tag_ops[1].op_type, OpType::Delete);
    assert_eq!(tag_ops[1].fields.as_ref().unwrap()["tag_name"], "fast");
    assert_eq!(tag_ops[2].op_type, OpType::Create);
    assert_eq!(tag_ops[2].fields.as_ref().unwrap()["tag_name"], "slow");
}
