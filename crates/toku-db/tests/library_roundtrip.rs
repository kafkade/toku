//! Full-domain lossless round-trip + merge-semantics tests for the canonical
//! backup engine (ADR-012, issue #200).
//!
//! These run under `cargo test --workspace`, which is what the `Validate` CI
//! gate executes — no CI job change required.

use toku_db::{Database, LibraryIo, RestoreMode};

/// Seed one fully-populated library covering every table the backup carries.
/// Uses direct SQL so each column is set explicitly and losslessly.
fn seed_full_library(db: &Database) {
    let c = &db.conn;

    // Work + series parents.
    c.execute(
        "INSERT INTO works (id, title, original_language, first_published, created_at)
         VALUES ('w1', 'Dune (Work)', 'en', '1965', '2020-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO series (id, name, total_books) VALUES ('s1', 'Dune Saga', 6)",
        [],
    )
    .unwrap();

    // Book with every column populated, including tombstone provenance columns.
    c.execute(
        "INSERT INTO books
            (id, title, subtitle, description, page_count, pub_date, language,
             format, duration_minutes, cover_hash, work_id, status, rating,
             goodreads_id, calibre_id, created_at, updated_at,
             deleted_at, deleted_by_device, search_text)
         VALUES
            ('b1', 'Dune', 'Book One', 'Desert planet.', 412, '1965-08-01', 'en',
             'audiobook', 1260, 'covdeadbeef', 'w1', 'read', 9,
             'gr-555', 4242, '2021-01-01T00:00:00Z', '2021-02-01T00:00:00Z',
             NULL, NULL, 'Dune')",
        [],
    )
    .unwrap();
    // A soft-deleted book (tombstone), to prove tombstones round-trip.
    c.execute(
        "INSERT INTO books
            (id, title, format, status, created_at, updated_at,
             deleted_at, deleted_by_device, search_text)
         VALUES
            ('b2', 'Ghost', 'physical', 'want_to_read', '2021-01-01T00:00:00Z',
             '2021-03-01T00:00:00Z', '2021-03-02T00:00:00Z', 'device-A', 'Ghost')",
        [],
    )
    .unwrap();

    // Authors + roles.
    c.execute(
        "INSERT INTO authors (id, name, sort_name) VALUES
            ('a1', 'Frank Herbert', 'Herbert, Frank'),
            ('a2', 'Scott Brick', 'Brick, Scott')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO book_authors (book_id, author_id, role, position) VALUES
            ('b1', 'a1', 'author', 0),
            ('b1', 'a2', 'narrator', 1)",
        [],
    )
    .unwrap();

    // Multiple ISBNs (ISBN-10 and ISBN-13) — must all round-trip.
    c.execute(
        "INSERT INTO isbns (isbn, book_id) VALUES
            ('9780441172719', 'b1'),
            ('0441172717', 'b1')",
        [],
    )
    .unwrap();

    // book <-> series with position.
    c.execute(
        "INSERT INTO book_series (book_id, series_id, position) VALUES ('b1', 's1', '1')",
        [],
    )
    .unwrap();

    // Shelves: one normal, one smart with a filter definition.
    c.execute(
        "INSERT INTO shelves (id, name, is_smart, smart_filter, created_at) VALUES
            ('sh1', 'Favorites', 0, NULL, '2021-01-01T00:00:00Z'),
            ('sh2', 'Recent Sci-Fi', 1, '{\"genre\":\"sci-fi\"}', '2021-01-02T00:00:00Z')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO book_shelves (book_id, shelf_id) VALUES ('b1', 'sh1')",
        [],
    )
    .unwrap();

    // Typed tags.
    c.execute(
        "INSERT INTO tags (id, name, tag_type, created_at) VALUES
            ('t1', 'epic', 'genre', '2021-01-01T00:00:00Z'),
            ('t2', 'tense', 'mood', '2021-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO book_tags (book_id, tag_id) VALUES ('b1', 't1'), ('b1', 't2')",
        [],
    )
    .unwrap();

    // Reading session + progress (append-only facts).
    c.execute(
        "INSERT INTO reading_sessions
            (id, book_id, started_at, finished_at, start_page, end_page, rating, notes, created_at)
         VALUES
            ('rs1', 'b1', '2021-01-05T00:00:00Z', '2021-01-20T00:00:00Z', 1, 412, 9,
             'loved it', '2021-01-05T00:00:00Z')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO reading_progress
            (id, book_id, session_id, progress_type, value, note, logged_at, created_at)
         VALUES
            ('rp1', 'b1', 'rs1', 'page', 200, 'halfway', '2021-01-10T00:00:00Z',
             '2021-01-10T00:00:00Z')",
        [],
    )
    .unwrap();

    // Notes: one live, one tombstoned.
    c.execute(
        "INSERT INTO notes (id, book_id, content, deleted_at, deleted_by_device, created_at, updated_at)
         VALUES
            ('n1', 'b1', 'The spice must flow.', NULL, NULL, '2021-01-06T00:00:00Z', '2021-01-06T00:00:00Z'),
            ('n2', 'b1', 'obsolete', '2021-01-07T00:00:00Z', 'device-A', '2021-01-06T00:00:00Z', '2021-01-07T00:00:00Z')",
        [],
    )
    .unwrap();

    // Review.
    c.execute(
        "INSERT INTO reviews (id, book_id, content, rating, deleted_at, deleted_by_device, created_at, updated_at)
         VALUES ('rv1', 'b1', 'A masterpiece.', 9, NULL, NULL, '2021-01-21T00:00:00Z', '2021-01-21T00:00:00Z')",
        [],
    )
    .unwrap();

    // User settings.
    c.execute(
        "INSERT INTO user_settings (id, key, value, sync_hlc, updated_at) VALUES
            ('us1', 'theme', 'dark', '2021-01-01T00:00:00Z-0001-devA', '2021-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    // Provenance with sync_hlc + user override flag.
    c.execute(
        "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
         VALUES
            ('b1', 'title', 'user', '2021-02-01T00:00:00Z', 1, '2021-02-01T00:00:00Z-0001-devA'),
            ('b1', 'rating', 'goodreads', '2021-01-01T00:00:00Z', 0, '2021-01-01T00:00:00Z-0001-devA')",
        [],
    )
    .unwrap();

    // Entity HLC underpinning notes/reviews LWW.
    c.execute(
        "INSERT INTO sync_entity_hlc (entity_type, entity_id, field_name, sync_hlc, device_id)
         VALUES
            ('note', 'n1', 'content', '2021-01-06T00:00:00Z-0001-devA', 'devA'),
            ('review', 'rv1', 'content', '2021-01-21T00:00:00Z-0001-devA', 'devA')",
        [],
    )
    .unwrap();

    // File association (binary lives elsewhere; row must round-trip).
    c.execute(
        "INSERT INTO files (id, book_id, path, format, size_bytes, checksum, source, source_ref, created_at, updated_at)
         VALUES ('f1', 'b1', '/lib/dune.epub', 'epub', 1024, 'sha-abc123', 'calibre', 'orig/path', '2021-01-01T00:00:00Z', '2021-01-01T00:00:00Z')",
        [],
    )
    .unwrap();

    // Import log + link.
    c.execute(
        "INSERT INTO import_logs (id, source, file_path, started_at, finished_at, total_rows, imported, skipped, errors)
         VALUES ('imp1', 'goodreads', '/tmp/gr.csv', '2021-01-01T00:00:00Z', '2021-01-01T00:01:00Z', 10, 9, 1, 0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO import_books (import_id, book_id) VALUES ('imp1', 'b1')",
        [],
    )
    .unwrap();
}

#[test]
fn full_model_replace_roundtrip_is_structurally_equal() {
    let src = Database::open_in_memory().unwrap();
    seed_full_library(&src);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    // Restore verbatim into a fresh database.
    let dst = Database::open_in_memory().unwrap();
    LibraryIo::new(&dst)
        .restore_library(&exported, RestoreMode::Replace)
        .unwrap();
    let reexported = LibraryIo::new(&dst).export_library().unwrap();

    // Structural equality across every entity — this is the lossless guarantee.
    assert_eq!(exported.books, reexported.books, "books");
    assert_eq!(exported.authors, reexported.authors, "authors");
    assert_eq!(
        exported.book_authors, reexported.book_authors,
        "book_authors"
    );
    assert_eq!(exported.isbns, reexported.isbns, "isbns");
    assert_eq!(exported.works, reexported.works, "works");
    assert_eq!(exported.series, reexported.series, "series");
    assert_eq!(exported.book_series, reexported.book_series, "book_series");
    assert_eq!(exported.shelves, reexported.shelves, "shelves");
    assert_eq!(
        exported.book_shelves, reexported.book_shelves,
        "book_shelves"
    );
    assert_eq!(exported.tags, reexported.tags, "tags");
    assert_eq!(exported.book_tags, reexported.book_tags, "book_tags");
    assert_eq!(
        exported.reading_sessions, reexported.reading_sessions,
        "reading_sessions"
    );
    assert_eq!(
        exported.reading_progress, reexported.reading_progress,
        "reading_progress"
    );
    assert_eq!(exported.notes, reexported.notes, "notes");
    assert_eq!(exported.reviews, reexported.reviews, "reviews");
    assert_eq!(
        exported.user_settings, reexported.user_settings,
        "user_settings"
    );
    assert_eq!(
        exported.metadata_provenance, reexported.metadata_provenance,
        "metadata_provenance"
    );
    assert_eq!(exported.entity_hlc, reexported.entity_hlc, "entity_hlc");
    assert_eq!(exported.files, reexported.files, "files");
    assert_eq!(exported.import_logs, reexported.import_logs, "import_logs");
    assert_eq!(
        exported.import_books, reexported.import_books,
        "import_books"
    );

    // The whole struct is equal (guards against a field being missed above).
    assert_eq!(exported, reexported);
}

#[test]
fn merge_into_fresh_db_then_reexport_is_equal() {
    let src = Database::open_in_memory().unwrap();
    seed_full_library(&src);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    let dst = Database::open_in_memory().unwrap();
    LibraryIo::new(&dst)
        .restore_library(&exported, RestoreMode::Merge)
        .unwrap();
    let reexported = LibraryIo::new(&dst).export_library().unwrap();

    assert_eq!(exported, reexported);
}

#[test]
fn merge_is_idempotent() {
    let src = Database::open_in_memory().unwrap();
    seed_full_library(&src);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    let dst = Database::open_in_memory().unwrap();
    let io = LibraryIo::new(&dst);
    io.restore_library(&exported, RestoreMode::Merge).unwrap();
    let after_first = LibraryIo::new(&dst).export_library().unwrap();
    // Re-applying the same backup must be a no-op.
    io.restore_library(&exported, RestoreMode::Merge).unwrap();
    let after_second = LibraryIo::new(&dst).export_library().unwrap();

    assert_eq!(after_first, after_second);
    assert_eq!(exported, after_second);
}

#[test]
fn merge_never_clobbers_a_newer_local_user_edit() {
    // Local DB has a book whose title was edited by the user at a NEWER HLC.
    let local = Database::open_in_memory().unwrap();
    local
        .conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('b1', 'User Title', 'physical', 'read', '2021-01-01T00:00:00Z', '2021-05-01T00:00:00Z', 'User Title')",
            [],
        )
        .unwrap();
    local
        .conn
        .execute(
            "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
             VALUES ('b1', 'title', 'user', '2021-05-01T00:00:00Z', 1, '2021-05-01T00:00:00Z-0001-devLocal')",
            [],
        )
        .unwrap();

    // Incoming backup carries an OLDER title change for the same book.
    let src = Database::open_in_memory().unwrap();
    src.conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('b1', 'Old Title', 'physical', 'read', '2021-01-01T00:00:00Z', '2021-02-01T00:00:00Z', 'Old Title')",
            [],
        )
        .unwrap();
    src.conn
        .execute(
            "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
             VALUES ('b1', 'title', 'goodreads', '2021-02-01T00:00:00Z', 0, '2021-02-01T00:00:00Z-0001-devA')",
            [],
        )
        .unwrap();
    let exported = LibraryIo::new(&src).export_library().unwrap();

    LibraryIo::new(&local)
        .restore_library(&exported, RestoreMode::Merge)
        .unwrap();

    let title: String = local
        .conn
        .query_row("SELECT title FROM books WHERE id = 'b1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        title, "User Title",
        "newer local user edit must survive merge"
    );
}

#[test]
fn merge_applies_a_newer_incoming_field() {
    // Local has an OLD field; incoming backup has a NEWER change → applied.
    let local = Database::open_in_memory().unwrap();
    local
        .conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('b1', 'Old', 'physical', 'read', '2021-01-01T00:00:00Z', '2021-01-01T00:00:00Z', 'Old')",
            [],
        )
        .unwrap();
    local
        .conn
        .execute(
            "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
             VALUES ('b1', 'title', 'goodreads', '2021-01-01T00:00:00Z', 0, '2021-01-01T00:00:00Z-0001-devA')",
            [],
        )
        .unwrap();

    let src = Database::open_in_memory().unwrap();
    src.conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('b1', 'New', 'physical', 'read', '2021-01-01T00:00:00Z', '2021-06-01T00:00:00Z', 'New')",
            [],
        )
        .unwrap();
    src.conn
        .execute(
            "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
             VALUES ('b1', 'title', 'user', '2021-06-01T00:00:00Z', 1, '2021-06-01T00:00:00Z-0001-devB')",
            [],
        )
        .unwrap();
    let exported = LibraryIo::new(&src).export_library().unwrap();

    LibraryIo::new(&local)
        .restore_library(&exported, RestoreMode::Merge)
        .unwrap();

    let title: String = local
        .conn
        .query_row("SELECT title FROM books WHERE id = 'b1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "New", "newer incoming field must be applied");
}

#[test]
fn merge_sessions_are_append_only_insert_if_absent() {
    let src = Database::open_in_memory().unwrap();
    seed_full_library(&src);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    // Destination already has the same book and session id.
    let dst = Database::open_in_memory().unwrap();
    let io = LibraryIo::new(&dst);
    io.restore_library(&exported, RestoreMode::Merge).unwrap();

    // Applying again must not duplicate the append-only session.
    let res = io.restore_library(&exported, RestoreMode::Merge).unwrap();
    assert_eq!(
        res.reading_sessions, 0,
        "existing session must not re-insert"
    );

    let n: i64 = dst
        .conn
        .query_row("SELECT COUNT(*) FROM reading_sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn replace_clears_existing_rows_before_restoring() {
    // Destination starts with an unrelated book that must be gone after replace.
    let dst = Database::open_in_memory().unwrap();
    dst.conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('zzz', 'Should Vanish', 'physical', 'read', '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z', 'Should Vanish')",
            [],
        )
        .unwrap();

    let src = Database::open_in_memory().unwrap();
    seed_full_library(&src);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    LibraryIo::new(&dst)
        .restore_library(&exported, RestoreMode::Replace)
        .unwrap();

    let gone: i64 = dst
        .conn
        .query_row("SELECT COUNT(*) FROM books WHERE id = 'zzz'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(gone, 0, "replace must clear pre-existing rows");

    let reexported = LibraryIo::new(&dst).export_library().unwrap();
    assert_eq!(exported, reexported);
}

#[test]
fn merge_dedups_book_by_isbn_and_remaps_joins() {
    // Local book has a different id but a shared ISBN → incoming must merge into
    // it rather than create a duplicate, and join rows must remap to local id.
    let local = Database::open_in_memory().unwrap();
    local
        .conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('local-id', 'Dune', 'physical', 'read', '2021-01-01T00:00:00Z', '2021-01-01T00:00:00Z', 'Dune')",
            [],
        )
        .unwrap();
    local
        .conn
        .execute(
            "INSERT INTO isbns (isbn, book_id) VALUES ('9780441172719', 'local-id')",
            [],
        )
        .unwrap();

    let src = Database::open_in_memory().unwrap();
    src.conn
        .execute(
            "INSERT INTO books (id, title, format, status, created_at, updated_at, search_text)
             VALUES ('incoming-id', 'Dune', 'physical', 'read', '2021-01-01T00:00:00Z', '2021-01-01T00:00:00Z', 'Dune')",
            [],
        )
        .unwrap();
    src.conn
        .execute(
            "INSERT INTO isbns (isbn, book_id) VALUES ('9780441172719', 'incoming-id')",
            [],
        )
        .unwrap();
    src.conn
        .execute("INSERT INTO tags (id, name, tag_type, created_at) VALUES ('t9', 'epic', 'genre', '2021-01-01T00:00:00Z')", [])
        .unwrap();
    src.conn
        .execute(
            "INSERT INTO book_tags (book_id, tag_id) VALUES ('incoming-id', 't9')",
            [],
        )
        .unwrap();
    let exported = LibraryIo::new(&src).export_library().unwrap();

    LibraryIo::new(&local)
        .restore_library(&exported, RestoreMode::Merge)
        .unwrap();

    let books: i64 = local
        .conn
        .query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
        .unwrap();
    assert_eq!(books, 1, "ISBN match must not duplicate the book");

    // The incoming tag link must have been remapped onto the local book id.
    let linked: i64 = local
        .conn
        .query_row(
            "SELECT COUNT(*) FROM book_tags WHERE book_id = 'local-id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(linked, 1, "join row must remap to the local book id");
}
