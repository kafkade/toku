//! Integration tests for disk organization (issue #152).

use std::path::PathBuf;

use toku_core::{Author, Book};
use toku_db::{BookRepository, Database};
use toku_files::{
    EbookFile, FileFormat, FileRepository, PathTemplate, PlanAction, apply_plan, plan_organize,
    sha256_file,
};

/// Seed a book with a single author and return the book id.
fn seed_book(db: &Database, title: &str, author: &str) -> uuid::Uuid {
    let repo = BookRepository::new(db);
    let book = Book::new(title);
    repo.create_book(&book).unwrap();
    let a = Author::new(author);
    repo.add_book_author(&a, &book.id, toku_core::ContributorRole::Author, 0)
        .unwrap();
    book.id
}

/// Link an on-disk file to a book and return the created record.
fn link_file(db: &Database, book_id: uuid::Uuid, path: &std::path::Path) -> EbookFile {
    let files = FileRepository::new(db);
    let format = FileFormat::from_path(path).unwrap();
    let size = std::fs::metadata(path).unwrap().len() as i64;
    let sum = sha256_file(path).unwrap();
    let f = EbookFile::new(
        book_id,
        path.to_string_lossy().to_string(),
        format,
        size,
        sum,
    );
    files.add_file(&f).unwrap();
    f
}

fn stored_path(db: &Database, book_id: &uuid::Uuid, format: FileFormat) -> String {
    FileRepository::new(db)
        .list_files(book_id)
        .unwrap()
        .into_iter()
        .find(|f| f.format == format)
        .unwrap()
        .path
}

#[test]
fn move_updates_db_path_and_relocates_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db, "Dune", "Frank Herbert");

    let src = dir.path().join("dune.epub");
    std::fs::write(&src, b"epub bytes").unwrap();
    link_file(&db, book_id, &src);

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();
    let plan = plan_organize(&db, &[book_id], &root, &tmpl, false).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].action, PlanAction::Move);

    let summary = apply_plan(&db, &plan).unwrap();
    assert_eq!(summary.moved, 1);
    assert_eq!(summary.copied, 0);

    let expected = root.join("Frank Herbert").join("Dune.epub");
    assert!(expected.exists(), "file should be at target");
    assert!(!src.exists(), "original should be gone after move");
    assert_eq!(
        stored_path(&db, &book_id, FileFormat::Epub),
        expected.to_string_lossy()
    );
}

#[test]
fn copy_leaves_original_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db, "Dune", "Frank Herbert");

    let src = dir.path().join("dune.epub");
    std::fs::write(&src, b"epub bytes").unwrap();
    link_file(&db, book_id, &src);

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();
    let plan = plan_organize(&db, &[book_id], &root, &tmpl, true).unwrap();
    assert_eq!(plan[0].action, PlanAction::Copy);

    let summary = apply_plan(&db, &plan).unwrap();
    assert_eq!(summary.copied, 1);

    let expected = root.join("Frank Herbert").join("Dune.epub");
    assert!(expected.exists());
    assert!(src.exists(), "copy must leave original in place");
    assert_eq!(
        stored_path(&db, &book_id, FileFormat::Epub),
        expected.to_string_lossy()
    );
}

#[test]
fn dry_run_plan_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db, "Dune", "Frank Herbert");

    let src = dir.path().join("dune.epub");
    std::fs::write(&src, b"epub bytes").unwrap();
    link_file(&db, book_id, &src);

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();
    // Building the plan is the dry run: we simply do not call apply_plan.
    let plan = plan_organize(&db, &[book_id], &root, &tmpl, false).unwrap();
    assert_eq!(plan.len(), 1);

    assert!(src.exists(), "source must be untouched by planning");
    assert!(
        !root.exists(),
        "library root must not be created by planning"
    );
    assert_eq!(
        stored_path(&db, &book_id, FileFormat::Epub),
        src.to_string_lossy()
    );
}

#[test]
fn rerun_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db, "Dune", "Frank Herbert");

    let src = dir.path().join("dune.epub");
    std::fs::write(&src, b"epub bytes").unwrap();
    link_file(&db, book_id, &src);

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();

    let plan = plan_organize(&db, &[book_id], &root, &tmpl, false).unwrap();
    apply_plan(&db, &plan).unwrap();

    // Second run: the file is already at its target, so nothing is actionable.
    let plan2 = plan_organize(&db, &[book_id], &root, &tmpl, false).unwrap();
    assert_eq!(plan2.len(), 1);
    assert!(matches!(plan2[0].action, PlanAction::Skip { .. }));

    let summary = apply_plan(&db, &plan2).unwrap();
    assert_eq!(summary.moved, 0);
    assert_eq!(summary.copied, 0);
    assert_eq!(summary.skipped, 1);
}

#[test]
fn collision_appends_numeric_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();

    // Two different books rendering to the same target folder+name.
    let b1 = seed_book(&db, "Dune", "Frank Herbert");
    let b2 = seed_book(&db, "Dune", "Frank Herbert");

    let s1 = dir.path().join("a.epub");
    std::fs::write(&s1, b"first").unwrap();
    link_file(&db, b1, &s1);
    let s2 = dir.path().join("b.epub");
    std::fs::write(&s2, b"second-different").unwrap();
    link_file(&db, b2, &s2);

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();
    let plan = plan_organize(&db, &[b1, b2], &root, &tmpl, false).unwrap();
    assert_eq!(plan.len(), 2);

    apply_plan(&db, &plan).unwrap();

    let base = root.join("Frank Herbert").join("Dune.epub");
    let suffixed = root.join("Frank Herbert").join("Dune (2).epub");
    assert!(base.exists());
    assert!(suffixed.exists(), "collision should produce a (2) file");
}

#[test]
fn missing_source_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db, "Dune", "Frank Herbert");

    // Record points at a non-existent path.
    let files = FileRepository::new(&db);
    let ghost = dir.path().join("ghost.epub");
    let f = EbookFile::new(
        book_id,
        ghost.to_string_lossy().to_string(),
        FileFormat::Epub,
        1,
        "deadbeef".to_string(),
    );
    files.add_file(&f).unwrap();

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();
    let plan = plan_organize(&db, &[book_id], &root, &tmpl, false).unwrap();
    assert!(matches!(plan[0].action, PlanAction::Skip { .. }));

    let summary = apply_plan(&db, &plan).unwrap();
    assert_eq!(summary.skipped, 1);
    assert!(!root.exists());
}

#[test]
fn unknown_author_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();

    // Book with no author.
    let repo = BookRepository::new(&db);
    let book = Book::new("Beowulf");
    repo.create_book(&book).unwrap();

    let src = dir.path().join("beowulf.epub");
    std::fs::write(&src, b"old english").unwrap();
    link_file(&db, book.id, &src);

    let root = dir.path().join("library");
    let tmpl = PathTemplate::parse("{author}/{title}.{format}").unwrap();
    let plan = plan_organize(&db, &[book.id], &root, &tmpl, false).unwrap();

    let expected: PathBuf = root.join("Unknown Author").join("Beowulf.epub");
    assert_eq!(plan[0].to, expected);
}
