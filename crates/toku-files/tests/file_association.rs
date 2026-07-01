use toku_core::Book;
use toku_db::{BookRepository, Database};
use toku_files::{EbookFile, FileFormat, FileRepository, sha256_file};

fn seed_book(db: &Database) -> uuid::Uuid {
    let repo = BookRepository::new(db);
    let book = Book::new("Dune");
    repo.create_book(&book).unwrap();
    book.id
}

#[test]
fn add_list_remove_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db);
    let files = FileRepository::new(&db);

    let epub = dir.path().join("dune.epub");
    std::fs::write(&epub, b"epub bytes").unwrap();
    let pdf = dir.path().join("dune.pdf");
    std::fs::write(&pdf, b"pdf bytes").unwrap();

    let f1 = EbookFile::new(
        book_id,
        epub.to_string_lossy().to_string(),
        FileFormat::Epub,
        10,
        sha256_file(&epub).unwrap(),
    );
    let f2 = EbookFile::new(
        book_id,
        pdf.to_string_lossy().to_string(),
        FileFormat::Pdf,
        9,
        sha256_file(&pdf).unwrap(),
    );
    files.add_file(&f1).unwrap();
    files.add_file(&f2).unwrap();

    let listed = files.list_files(&book_id).unwrap();
    assert_eq!(listed.len(), 2);

    let removed = files.remove_by_format(&book_id, FileFormat::Epub).unwrap();
    assert!(removed.is_some());
    assert_eq!(files.list_files(&book_id).unwrap().len(), 1);

    let removed = files.remove_by_path(&book_id, &f2.path).unwrap();
    assert!(removed.is_some());
    assert!(files.list_files(&book_id).unwrap().is_empty());
}

#[test]
fn duplicate_checksum_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db);
    let files = FileRepository::new(&db);

    let p = dir.path().join("a.epub");
    std::fs::write(&p, b"same content").unwrap();
    let sum = sha256_file(&p).unwrap();

    let f = EbookFile::new(
        book_id,
        p.to_string_lossy().to_string(),
        FileFormat::Epub,
        12,
        sum.clone(),
    );
    files.add_file(&f).unwrap();

    let dup = EbookFile::new(
        book_id,
        "/elsewhere/a.epub".into(),
        FileFormat::Epub,
        12,
        sum,
    );
    assert!(files.add_file(&dup).is_err());
}

#[test]
fn provenance_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let book_id = seed_book(&db);
    let files = FileRepository::new(&db);

    // Default record: user-added, no external reference.
    let user_epub = dir.path().join("user.epub");
    std::fs::write(&user_epub, b"user bytes").unwrap();
    let f_user = EbookFile::new(
        book_id,
        user_epub.to_string_lossy().to_string(),
        FileFormat::Epub,
        10,
        sha256_file(&user_epub).unwrap(),
    );
    assert_eq!(f_user.source, "user");
    assert_eq!(f_user.source_ref, None);

    // Importer-created record with provenance.
    let cal_pdf = dir.path().join("calibre.pdf");
    std::fs::write(&cal_pdf, b"calibre bytes").unwrap();
    let f_cal = EbookFile::new(
        book_id,
        cal_pdf.to_string_lossy().to_string(),
        FileFormat::Pdf,
        13,
        sha256_file(&cal_pdf).unwrap(),
    )
    .with_source("calibre", Some("/library/Calibre/dune.pdf".to_string()));

    files.add_file(&f_user).unwrap();
    files.add_file(&f_cal).unwrap();

    let listed = files.list_files(&book_id).unwrap();
    assert_eq!(listed.len(), 2);

    let epub = listed
        .iter()
        .find(|f| f.format == FileFormat::Epub)
        .unwrap();
    assert_eq!(epub.source, "user");
    assert_eq!(epub.source_ref, None);

    let pdf = listed.iter().find(|f| f.format == FileFormat::Pdf).unwrap();
    assert_eq!(pdf.source, "calibre");
    assert_eq!(pdf.source_ref.as_deref(), Some("/library/Calibre/dune.pdf"));
}
