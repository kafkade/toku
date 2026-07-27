//! Full-domain library export and restore (ADR-012).
//!
//! [`LibraryIo::export_library`] reads every persisted, user-owned table into
//! the shared [`LibraryData`] schema. [`LibraryIo::restore_library`] writes it
//! back with either merge (default, idempotent, precedence-respecting) or
//! replace (verbatim into a cleared library) semantics.
//!
//! Binaries (cover images and ebook files) live outside this module: the
//! backup container in `toku-export` reads/writes them content-addressed, using
//! the `cover_hash` and `files.checksum` values carried here.

use rusqlite::{Row, params};
use toku_core::backup_schema::*;

use crate::{Database, DbError};

/// Restore strategy for [`LibraryIo::restore_library`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    /// Additive, precedence-respecting merge into a possibly non-empty library.
    Merge,
    /// Verbatim restore into a cleared library (disaster recovery / fresh DB).
    Replace,
}

/// Per-entity counts produced by a restore.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreResult {
    pub books_inserted: usize,
    pub books_updated: usize,
    pub authors: usize,
    pub reading_sessions: usize,
    pub reading_progress: usize,
    pub notes: usize,
    pub reviews: usize,
    pub tags: usize,
    pub shelves: usize,
    pub works: usize,
    pub series: usize,
    pub isbns: usize,
    pub files: usize,
    pub import_logs: usize,
}

/// Full-domain (de)serialization over a [`Database`].
pub struct LibraryIo<'a> {
    db: &'a Database,
}

impl<'a> LibraryIo<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    // -----------------------------------------------------------------------
    // Export
    // -----------------------------------------------------------------------

    /// Read the entire library into the shared [`LibraryData`] schema.
    pub fn export_library(&self) -> Result<LibraryData, DbError> {
        let conn = &self.db.conn;
        Ok(LibraryData {
            books: query_vec(
                conn,
                "SELECT id, title, subtitle, description, page_count, pub_date, language,
                        format, duration_minutes, cover_hash, work_id, status, rating,
                        goodreads_id, calibre_id, created_at, updated_at,
                        deleted_at, deleted_by_device
                 FROM books ORDER BY id",
                row_to_book,
            )?,
            authors: query_vec(
                conn,
                "SELECT id, name, sort_name FROM authors ORDER BY id",
                |r| {
                    Ok(AuthorRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        sort_name: r.get(2)?,
                    })
                },
            )?,
            book_authors: query_vec(
                conn,
                "SELECT book_id, author_id, role, position FROM book_authors
                 ORDER BY book_id, position, author_id",
                |r| {
                    Ok(BookAuthorRow {
                        book_id: r.get(0)?,
                        author_id: r.get(1)?,
                        role: r.get(2)?,
                        position: r.get(3)?,
                    })
                },
            )?,
            isbns: query_vec(conn, "SELECT isbn, book_id FROM isbns ORDER BY isbn", |r| {
                Ok(IsbnRow {
                    isbn: r.get(0)?,
                    book_id: r.get(1)?,
                })
            })?,
            works: query_vec(
                conn,
                "SELECT id, title, original_language, first_published, created_at
                 FROM works ORDER BY id",
                |r| {
                    Ok(WorkRow {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        original_language: r.get(2)?,
                        first_published: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                },
            )?,
            series: query_vec(
                conn,
                "SELECT id, name, total_books FROM series ORDER BY id",
                |r| {
                    Ok(SeriesRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        total_books: r.get(2)?,
                    })
                },
            )?,
            book_series: query_vec(
                conn,
                "SELECT book_id, series_id, position FROM book_series
                 ORDER BY book_id, series_id",
                |r| {
                    Ok(BookSeriesRow {
                        book_id: r.get(0)?,
                        series_id: r.get(1)?,
                        position: r.get(2)?,
                    })
                },
            )?,
            shelves: query_vec(
                conn,
                "SELECT id, name, is_smart, smart_filter, created_at FROM shelves ORDER BY id",
                |r| {
                    Ok(ShelfRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        is_smart: r.get::<_, i64>(2)? != 0,
                        smart_filter: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                },
            )?,
            book_shelves: query_vec(
                conn,
                "SELECT book_id, shelf_id FROM book_shelves ORDER BY book_id, shelf_id",
                |r| {
                    Ok(BookShelfRow {
                        book_id: r.get(0)?,
                        shelf_id: r.get(1)?,
                    })
                },
            )?,
            tags: query_vec(
                conn,
                "SELECT id, name, tag_type, created_at FROM tags ORDER BY id",
                |r| {
                    Ok(TagRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        tag_type: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )?,
            book_tags: query_vec(
                conn,
                "SELECT book_id, tag_id FROM book_tags ORDER BY book_id, tag_id",
                |r| {
                    Ok(BookTagRow {
                        book_id: r.get(0)?,
                        tag_id: r.get(1)?,
                    })
                },
            )?,
            reading_sessions: query_vec(
                conn,
                "SELECT id, book_id, started_at, finished_at, start_page, end_page,
                        rating, notes, created_at
                 FROM reading_sessions ORDER BY id",
                |r| {
                    Ok(ReadingSessionRow {
                        id: r.get(0)?,
                        book_id: r.get(1)?,
                        started_at: r.get(2)?,
                        finished_at: r.get(3)?,
                        start_page: r.get(4)?,
                        end_page: r.get(5)?,
                        rating: r.get(6)?,
                        notes: r.get(7)?,
                        created_at: r.get(8)?,
                    })
                },
            )?,
            reading_progress: query_vec(
                conn,
                "SELECT id, book_id, session_id, progress_type, value, note,
                        logged_at, created_at
                 FROM reading_progress ORDER BY id",
                |r| {
                    Ok(ReadingProgressRow {
                        id: r.get(0)?,
                        book_id: r.get(1)?,
                        session_id: r.get(2)?,
                        progress_type: r.get(3)?,
                        value: r.get(4)?,
                        note: r.get(5)?,
                        logged_at: r.get(6)?,
                        created_at: r.get(7)?,
                    })
                },
            )?,
            notes: query_vec(
                conn,
                "SELECT id, book_id, content, deleted_at, deleted_by_device,
                        created_at, updated_at
                 FROM notes ORDER BY id",
                |r| {
                    Ok(NoteRow {
                        id: r.get(0)?,
                        book_id: r.get(1)?,
                        content: r.get(2)?,
                        deleted_at: r.get(3)?,
                        deleted_by_device: r.get(4)?,
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                    })
                },
            )?,
            reviews: query_vec(
                conn,
                "SELECT id, book_id, content, rating, deleted_at, deleted_by_device,
                        created_at, updated_at
                 FROM reviews ORDER BY id",
                |r| {
                    Ok(ReviewRow {
                        id: r.get(0)?,
                        book_id: r.get(1)?,
                        content: r.get(2)?,
                        rating: r.get(3)?,
                        deleted_at: r.get(4)?,
                        deleted_by_device: r.get(5)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                },
            )?,
            user_settings: query_vec(
                conn,
                "SELECT id, key, value, sync_hlc, updated_at FROM user_settings ORDER BY id",
                |r| {
                    Ok(UserSettingRow {
                        id: r.get(0)?,
                        key: r.get(1)?,
                        value: r.get(2)?,
                        sync_hlc: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )?,
            metadata_provenance: query_vec(
                conn,
                "SELECT book_id, field_name, source, source_date, is_user_override, sync_hlc
                 FROM metadata_provenance ORDER BY book_id, field_name",
                |r| {
                    Ok(ProvenanceRow {
                        book_id: r.get(0)?,
                        field_name: r.get(1)?,
                        source: r.get(2)?,
                        source_date: r.get(3)?,
                        is_user_override: r.get::<_, i64>(4)? != 0,
                        sync_hlc: r.get(5)?,
                    })
                },
            )?,
            entity_hlc: query_vec(
                conn,
                "SELECT entity_type, entity_id, field_name, sync_hlc, device_id
                 FROM sync_entity_hlc ORDER BY entity_type, entity_id, field_name",
                |r| {
                    Ok(EntityHlcRow {
                        entity_type: r.get(0)?,
                        entity_id: r.get(1)?,
                        field_name: r.get(2)?,
                        sync_hlc: r.get(3)?,
                        device_id: r.get(4)?,
                    })
                },
            )?,
            files: query_vec(
                conn,
                "SELECT id, book_id, path, format, size_bytes, checksum, source,
                        source_ref, created_at, updated_at
                 FROM files ORDER BY id",
                |r| {
                    Ok(FileRow {
                        id: r.get(0)?,
                        book_id: r.get(1)?,
                        path: r.get(2)?,
                        format: r.get(3)?,
                        size_bytes: r.get(4)?,
                        checksum: r.get(5)?,
                        source: r.get(6)?,
                        source_ref: r.get(7)?,
                        created_at: r.get(8)?,
                        updated_at: r.get(9)?,
                    })
                },
            )?,
            import_logs: query_vec(
                conn,
                "SELECT id, source, file_path, started_at, finished_at,
                        total_rows, imported, skipped, errors
                 FROM import_logs ORDER BY id",
                |r| {
                    Ok(ImportLogRow {
                        id: r.get(0)?,
                        source: r.get(1)?,
                        file_path: r.get(2)?,
                        started_at: r.get(3)?,
                        finished_at: r.get(4)?,
                        total_rows: r.get(5)?,
                        imported: r.get(6)?,
                        skipped: r.get(7)?,
                        errors: r.get(8)?,
                    })
                },
            )?,
            import_books: query_vec(
                conn,
                "SELECT import_id, book_id FROM import_books ORDER BY import_id, book_id",
                |r| {
                    Ok(ImportBookRow {
                        import_id: r.get(0)?,
                        book_id: r.get(1)?,
                    })
                },
            )?,
        })
    }

    // -----------------------------------------------------------------------
    // Restore
    // -----------------------------------------------------------------------

    /// Write [`LibraryData`] into the database with the given [`RestoreMode`].
    ///
    /// Runs in a single transaction: either the whole restore commits or the
    /// database is left untouched.
    pub fn restore_library(
        &self,
        data: &LibraryData,
        mode: RestoreMode,
    ) -> Result<RestoreResult, DbError> {
        let tx = self.db.conn.unchecked_transaction()?;
        let mut result = RestoreResult::default();

        if mode == RestoreMode::Replace {
            clear_library(&tx)?;
        }

        // Independent parent entities first (dedup by natural key where the
        // schema has a UNIQUE constraint, so join rows can be remapped).
        let mut book_map = IdMap::new();
        let mut tag_map = IdMap::new();
        let mut shelf_map = IdMap::new();

        // Books (and their metadata provenance, applied together for LWW).
        restore_books(&tx, data, mode, &mut book_map, &mut result)?;

        // Authors, works, series: insert-if-absent by id (identity remap).
        for a in &data.authors {
            let n = tx.execute(
                "INSERT OR IGNORE INTO authors (id, name, sort_name) VALUES (?1, ?2, ?3)",
                params![a.id, a.name, a.sort_name],
            )?;
            result.authors += n;
        }
        for w in &data.works {
            let n = tx.execute(
                "INSERT OR IGNORE INTO works
                    (id, title, original_language, first_published, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    w.id,
                    w.title,
                    w.original_language,
                    w.first_published,
                    w.created_at
                ],
            )?;
            result.works += n;
        }
        for s in &data.series {
            let n = tx.execute(
                "INSERT OR IGNORE INTO series (id, name, total_books) VALUES (?1, ?2, ?3)",
                params![s.id, s.name, s.total_books],
            )?;
            result.series += n;
        }

        // Tags: dedup by (name, tag_type) UNIQUE.
        for t in &data.tags {
            let local_id: Option<String> = tx
                .query_row(
                    "SELECT id FROM tags WHERE name = ?1 AND tag_type = ?2",
                    params![t.name, t.tag_type],
                    |r| r.get(0),
                )
                .optional_dberr()?;
            match local_id {
                Some(id) => tag_map.set(&t.id, &id),
                None => {
                    tx.execute(
                        "INSERT INTO tags (id, name, tag_type, created_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![t.id, t.name, t.tag_type, t.created_at],
                    )?;
                    tag_map.set(&t.id, &t.id);
                    result.tags += 1;
                }
            }
        }

        // Shelves: dedup by name UNIQUE.
        for s in &data.shelves {
            let local_id: Option<String> = tx
                .query_row(
                    "SELECT id FROM shelves WHERE name = ?1",
                    params![s.name],
                    |r| r.get(0),
                )
                .optional_dberr()?;
            match local_id {
                Some(id) => shelf_map.set(&s.id, &id),
                None => {
                    tx.execute(
                        "INSERT INTO shelves (id, name, is_smart, smart_filter, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            s.id,
                            s.name,
                            i64::from(s.is_smart),
                            s.smart_filter,
                            s.created_at
                        ],
                    )?;
                    shelf_map.set(&s.id, &s.id);
                    result.shelves += 1;
                }
            }
        }

        // Join rows: insert-if-absent, remapping ids that may have deduped.
        for ba in &data.book_authors {
            let book_id = book_map.get(&ba.book_id);
            tx.execute(
                "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![book_id, ba.author_id, ba.role, ba.position],
            )?;
        }
        for i in &data.isbns {
            let book_id = book_map.get(&i.book_id);
            let n = tx.execute(
                "INSERT OR IGNORE INTO isbns (isbn, book_id) VALUES (?1, ?2)",
                params![i.isbn, book_id],
            )?;
            result.isbns += n;
        }
        for bs in &data.book_series {
            let book_id = book_map.get(&bs.book_id);
            tx.execute(
                "INSERT OR IGNORE INTO book_series (book_id, series_id, position)
                 VALUES (?1, ?2, ?3)",
                params![book_id, bs.series_id, bs.position],
            )?;
        }
        for bsh in &data.book_shelves {
            let book_id = book_map.get(&bsh.book_id);
            let shelf_id = shelf_map.get(&bsh.shelf_id);
            tx.execute(
                "INSERT OR IGNORE INTO book_shelves (book_id, shelf_id) VALUES (?1, ?2)",
                params![book_id, shelf_id],
            )?;
        }
        for bt in &data.book_tags {
            let book_id = book_map.get(&bt.book_id);
            let tag_id = tag_map.get(&bt.tag_id);
            tx.execute(
                "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
                params![book_id, tag_id],
            )?;
        }

        // Append-only facts: sessions then progress (progress may reference a
        // session). Insert-if-absent by id.
        for s in &data.reading_sessions {
            let book_id = book_map.get(&s.book_id);
            let n = tx.execute(
                "INSERT OR IGNORE INTO reading_sessions
                    (id, book_id, started_at, finished_at, start_page, end_page,
                     rating, notes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    s.id,
                    book_id,
                    s.started_at,
                    s.finished_at,
                    s.start_page,
                    s.end_page,
                    s.rating,
                    s.notes,
                    s.created_at
                ],
            )?;
            result.reading_sessions += n;
        }
        for p in &data.reading_progress {
            let book_id = book_map.get(&p.book_id);
            let n = tx.execute(
                "INSERT OR IGNORE INTO reading_progress
                    (id, book_id, session_id, progress_type, value, note,
                     logged_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    p.id,
                    book_id,
                    p.session_id,
                    p.progress_type,
                    p.value,
                    p.note,
                    p.logged_at,
                    p.created_at
                ],
            )?;
            result.reading_progress += n;
        }

        // Entity HLC rows underpin notes/reviews LWW; write them before so the
        // per-field comparisons below see incoming clocks.
        restore_entity_hlc(&tx, data, &book_map)?;

        // Notes / reviews: LWW by HLC honouring tombstones.
        result.notes = restore_notes(&tx, data, mode, &book_map)?;
        result.reviews = restore_reviews(&tx, data, mode, &book_map)?;

        // Settings: LWW by key.
        restore_settings(&tx, data, mode)?;

        // Files: insert-if-absent by id (binary dedup handled by the container).
        for f in &data.files {
            let book_id = book_map.get(&f.book_id);
            let n = tx.execute(
                "INSERT OR IGNORE INTO files
                    (id, book_id, path, format, size_bytes, checksum, source,
                     source_ref, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    f.id,
                    book_id,
                    f.path,
                    f.format,
                    f.size_bytes,
                    f.checksum,
                    f.source,
                    f.source_ref,
                    f.created_at,
                    f.updated_at
                ],
            )?;
            result.files += n;
        }

        // Import logs and their book links (insert-if-absent).
        for l in &data.import_logs {
            let n = tx.execute(
                "INSERT OR IGNORE INTO import_logs
                    (id, source, file_path, started_at, finished_at,
                     total_rows, imported, skipped, errors)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    l.id,
                    l.source,
                    l.file_path,
                    l.started_at,
                    l.finished_at,
                    l.total_rows,
                    l.imported,
                    l.skipped,
                    l.errors
                ],
            )?;
            result.import_logs += n;
        }
        for ib in &data.import_books {
            let book_id = book_map.get(&ib.book_id);
            tx.execute(
                "INSERT OR IGNORE INTO import_books (import_id, book_id) VALUES (?1, ?2)",
                params![ib.import_id, book_id],
            )?;
        }

        // Recompute FTS search_text (including author names) for restored books.
        tx.execute(
            "UPDATE books SET search_text = TRIM(
                COALESCE(title, '') || ' ' || COALESCE(subtitle, '') || ' ' ||
                COALESCE(description, '') || ' ' ||
                COALESCE((SELECT GROUP_CONCAT(a.name, ' ')
                          FROM book_authors ba JOIN authors a ON a.id = ba.author_id
                          WHERE ba.book_id = books.id), ''))",
            [],
        )?;

        tx.commit()?;
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Books + provenance
// ---------------------------------------------------------------------------

fn restore_books(
    tx: &rusqlite::Transaction<'_>,
    data: &LibraryData,
    mode: RestoreMode,
    book_map: &mut IdMap,
    result: &mut RestoreResult,
) -> Result<(), DbError> {
    // Index provenance rows by incoming book id for quick per-book lookup.
    for book in &data.books {
        let existing = if mode == RestoreMode::Replace {
            None
        } else {
            find_existing_book(tx, book, data)?
        };

        match existing {
            None => {
                insert_book(tx, book)?;
                book_map.set(&book.id, &book.id);
                result.books_inserted += 1;
                // Copy provenance verbatim for a freshly inserted book.
                for pr in provenance_for(data, &book.id) {
                    upsert_provenance(tx, &book.id, pr)?;
                }
            }
            Some(local_id) => {
                book_map.set(&book.id, &local_id);
                let changed = merge_book_fields(tx, &local_id, book, data)?;
                if changed {
                    result.books_updated += 1;
                }
            }
        }
    }
    Ok(())
}

fn insert_book(tx: &rusqlite::Transaction<'_>, b: &BookRow) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO books
            (id, title, subtitle, description, page_count, pub_date, language,
             format, duration_minutes, cover_hash, work_id, status, rating,
             goodreads_id, calibre_id, created_at, updated_at,
             deleted_at, deleted_by_device, search_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            b.id,
            b.title,
            b.subtitle,
            b.description,
            b.page_count,
            b.pub_date,
            b.language,
            b.format,
            b.duration_minutes,
            b.cover_hash,
            b.work_id,
            b.status,
            b.rating,
            b.goodreads_id,
            b.calibre_id,
            b.created_at,
            b.updated_at,
            b.deleted_at,
            b.deleted_by_device,
            b.title
        ],
    )?;
    Ok(())
}

/// Match an incoming book to an existing local row: id, then source IDs, then
/// any shared ISBN (ADR-012 D3).
fn find_existing_book(
    tx: &rusqlite::Transaction<'_>,
    book: &BookRow,
    data: &LibraryData,
) -> Result<Option<String>, DbError> {
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM books WHERE id = ?1",
            params![book.id],
            |r| r.get::<_, String>(0),
        )
        .optional_dberr()?
    {
        return Ok(Some(id));
    }
    if let Some(gr) = &book.goodreads_id
        && let Some(id) = tx
            .query_row(
                "SELECT id FROM books WHERE goodreads_id = ?1",
                params![gr],
                |r| r.get::<_, String>(0),
            )
            .optional_dberr()?
    {
        return Ok(Some(id));
    }
    if let Some(cal) = book.calibre_id
        && let Some(id) = tx
            .query_row(
                "SELECT id FROM books WHERE calibre_id = ?1",
                params![cal],
                |r| r.get::<_, String>(0),
            )
            .optional_dberr()?
    {
        return Ok(Some(id));
    }
    for isbn in data.isbns.iter().filter(|i| i.book_id == book.id) {
        if let Some(id) = tx
            .query_row(
                "SELECT book_id FROM isbns WHERE isbn = ?1",
                params![isbn.isbn],
                |r| r.get::<_, String>(0),
            )
            .optional_dberr()?
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Apply incoming book fields to an existing row using per-field provenance
/// LWW: a field is applied only when its incoming HLC is strictly newer than
/// the local HLC, which inherently protects a newer local user edit.
fn merge_book_fields(
    tx: &rusqlite::Transaction<'_>,
    local_id: &str,
    incoming: &BookRow,
    data: &LibraryData,
) -> Result<bool, DbError> {
    let mut changed = false;
    for pr in provenance_for(data, &incoming.id) {
        let incoming_hlc = pr
            .sync_hlc
            .clone()
            .unwrap_or_else(|| pr.source_date.clone());
        let local_hlc: Option<String> = tx
            .query_row(
                "SELECT COALESCE(sync_hlc, source_date) FROM metadata_provenance
                 WHERE book_id = ?1 AND field_name = ?2",
                params![local_id, pr.field_name],
                |r| r.get(0),
            )
            .optional_dberr()?;
        let apply = match &local_hlc {
            None => true,
            Some(lh) => incoming_hlc > *lh,
        };
        if !apply {
            continue;
        }
        if apply_book_field(tx, local_id, &pr.field_name, incoming)? {
            changed = true;
        }
        upsert_provenance_at(tx, local_id, pr, &incoming_hlc)?;
    }
    Ok(changed)
}

/// Update a single book column identified by a provenance `field_name`.
fn apply_book_field(
    tx: &rusqlite::Transaction<'_>,
    local_id: &str,
    field: &str,
    b: &BookRow,
) -> Result<bool, DbError> {
    let n = match field {
        "title" => tx.execute(
            "UPDATE books SET title = ?1 WHERE id = ?2",
            params![b.title, local_id],
        )?,
        "subtitle" => tx.execute(
            "UPDATE books SET subtitle = ?1 WHERE id = ?2",
            params![b.subtitle, local_id],
        )?,
        "description" => tx.execute(
            "UPDATE books SET description = ?1 WHERE id = ?2",
            params![b.description, local_id],
        )?,
        "page_count" => tx.execute(
            "UPDATE books SET page_count = ?1 WHERE id = ?2",
            params![b.page_count, local_id],
        )?,
        "pub_date" => tx.execute(
            "UPDATE books SET pub_date = ?1 WHERE id = ?2",
            params![b.pub_date, local_id],
        )?,
        "language" => tx.execute(
            "UPDATE books SET language = ?1 WHERE id = ?2",
            params![b.language, local_id],
        )?,
        "format" => tx.execute(
            "UPDATE books SET format = ?1 WHERE id = ?2",
            params![b.format, local_id],
        )?,
        "duration_minutes" => tx.execute(
            "UPDATE books SET duration_minutes = ?1 WHERE id = ?2",
            params![b.duration_minutes, local_id],
        )?,
        "cover_hash" => tx.execute(
            "UPDATE books SET cover_hash = ?1 WHERE id = ?2",
            params![b.cover_hash, local_id],
        )?,
        "status" => tx.execute(
            "UPDATE books SET status = ?1 WHERE id = ?2",
            params![b.status, local_id],
        )?,
        "rating" => tx.execute(
            "UPDATE books SET rating = ?1 WHERE id = ?2",
            params![b.rating, local_id],
        )?,
        // Unknown/unmapped provenance field: nothing to update on the row.
        _ => 0,
    };
    // Keep updated_at monotonic with the applied change.
    if n > 0 {
        tx.execute(
            "UPDATE books SET updated_at = ?1 WHERE id = ?2",
            params![b.updated_at, local_id],
        )?;
    }
    Ok(n > 0)
}

fn provenance_for<'d>(
    data: &'d LibraryData,
    book_id: &str,
) -> impl Iterator<Item = &'d ProvenanceRow> {
    data.metadata_provenance
        .iter()
        .filter(move |p| p.book_id == book_id)
}

fn upsert_provenance(
    tx: &rusqlite::Transaction<'_>,
    book_id: &str,
    pr: &ProvenanceRow,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO metadata_provenance
            (book_id, field_name, source, source_date, is_user_override, sync_hlc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(book_id, field_name) DO UPDATE SET
            source = excluded.source, source_date = excluded.source_date,
            is_user_override = excluded.is_user_override, sync_hlc = excluded.sync_hlc",
        params![
            book_id,
            pr.field_name,
            pr.source,
            pr.source_date,
            i64::from(pr.is_user_override),
            pr.sync_hlc
        ],
    )?;
    Ok(())
}

fn upsert_provenance_at(
    tx: &rusqlite::Transaction<'_>,
    book_id: &str,
    pr: &ProvenanceRow,
    hlc: &str,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO metadata_provenance
            (book_id, field_name, source, source_date, is_user_override, sync_hlc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(book_id, field_name) DO UPDATE SET
            source = excluded.source, source_date = excluded.source_date,
            is_user_override = excluded.is_user_override, sync_hlc = excluded.sync_hlc",
        params![
            book_id,
            pr.field_name,
            pr.source,
            pr.source_date,
            i64::from(pr.is_user_override),
            hlc
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Notes / reviews / settings / entity HLC
// ---------------------------------------------------------------------------

fn restore_entity_hlc(
    tx: &rusqlite::Transaction<'_>,
    data: &LibraryData,
    book_map: &IdMap,
) -> Result<(), DbError> {
    for h in &data.entity_hlc {
        // Remap book-scoped entity ids (notes/reviews carry their own ids, so
        // only book entities need remapping).
        let entity_id = if h.entity_type == "book" {
            book_map.get(&h.entity_id)
        } else {
            h.entity_id.clone()
        };
        tx.execute(
            "INSERT INTO sync_entity_hlc
                (entity_type, entity_id, field_name, sync_hlc, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(entity_type, entity_id, field_name) DO UPDATE SET
                sync_hlc = excluded.sync_hlc, device_id = excluded.device_id
             WHERE excluded.sync_hlc > sync_entity_hlc.sync_hlc",
            params![
                h.entity_type,
                entity_id,
                h.field_name,
                h.sync_hlc,
                h.device_id
            ],
        )?;
    }
    Ok(())
}

fn entity_hlc(
    data: &LibraryData,
    entity_type: &str,
    entity_id: &str,
    field: &str,
) -> Option<String> {
    data.entity_hlc
        .iter()
        .find(|h| h.entity_type == entity_type && h.entity_id == entity_id && h.field_name == field)
        .map(|h| h.sync_hlc.clone())
}

fn local_entity_hlc(
    tx: &rusqlite::Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
    field: &str,
) -> Result<Option<String>, DbError> {
    tx.query_row(
        "SELECT sync_hlc FROM sync_entity_hlc
         WHERE entity_type = ?1 AND entity_id = ?2 AND field_name = ?3",
        params![entity_type, entity_id, field],
        |r| r.get(0),
    )
    .optional_dberr()
}

fn restore_notes(
    tx: &rusqlite::Transaction<'_>,
    data: &LibraryData,
    mode: RestoreMode,
    book_map: &IdMap,
) -> Result<usize, DbError> {
    let mut count = 0;
    for note in &data.notes {
        let book_id = book_map.get(&note.book_id);
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM notes WHERE id = ?1",
                params![note.id],
                |_| Ok(()),
            )
            .optional_dberr()?
            .is_some();

        if !exists {
            tx.execute(
                "INSERT INTO notes
                    (id, book_id, content, deleted_at, deleted_by_device,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    note.id,
                    book_id,
                    note.content,
                    note.deleted_at,
                    note.deleted_by_device,
                    note.created_at,
                    note.updated_at
                ],
            )?;
            count += 1;
        } else if mode == RestoreMode::Merge {
            let incoming = entity_hlc(data, "note", &note.id, "content");
            let local = local_entity_hlc(tx, "note", &note.id, "content")?;
            let apply = match (&incoming, &local) {
                (Some(i), Some(l)) => i > l,
                (Some(_), None) => true,
                // No incoming clock: fall back to updated_at LWW.
                (None, _) => is_newer_updated_at(tx, "notes", &note.id, &note.updated_at)?,
            };
            if apply {
                tx.execute(
                    "UPDATE notes SET content = ?1, deleted_at = ?2,
                        deleted_by_device = ?3, updated_at = ?4 WHERE id = ?5",
                    params![
                        note.content,
                        note.deleted_at,
                        note.deleted_by_device,
                        note.updated_at,
                        note.id
                    ],
                )?;
            }
        }
    }
    Ok(count)
}

fn restore_reviews(
    tx: &rusqlite::Transaction<'_>,
    data: &LibraryData,
    mode: RestoreMode,
    book_map: &IdMap,
) -> Result<usize, DbError> {
    let mut count = 0;
    for review in &data.reviews {
        let book_id = book_map.get(&review.book_id);
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM reviews WHERE id = ?1",
                params![review.id],
                |_| Ok(()),
            )
            .optional_dberr()?
            .is_some();

        if !exists {
            tx.execute(
                "INSERT INTO reviews
                    (id, book_id, content, rating, deleted_at, deleted_by_device,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    review.id,
                    book_id,
                    review.content,
                    review.rating,
                    review.deleted_at,
                    review.deleted_by_device,
                    review.created_at,
                    review.updated_at
                ],
            )?;
            count += 1;
        } else if mode == RestoreMode::Merge {
            let incoming = entity_hlc(data, "review", &review.id, "content");
            let local = local_entity_hlc(tx, "review", &review.id, "content")?;
            let apply = match (&incoming, &local) {
                (Some(i), Some(l)) => i > l,
                (Some(_), None) => true,
                (None, _) => is_newer_updated_at(tx, "reviews", &review.id, &review.updated_at)?,
            };
            if apply {
                tx.execute(
                    "UPDATE reviews SET content = ?1, rating = ?2, deleted_at = ?3,
                        deleted_by_device = ?4, updated_at = ?5 WHERE id = ?6",
                    params![
                        review.content,
                        review.rating,
                        review.deleted_at,
                        review.deleted_by_device,
                        review.updated_at,
                        review.id
                    ],
                )?;
            }
        }
    }
    Ok(count)
}

fn restore_settings(
    tx: &rusqlite::Transaction<'_>,
    data: &LibraryData,
    mode: RestoreMode,
) -> Result<(), DbError> {
    for s in &data.user_settings {
        let local: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT id, sync_hlc FROM user_settings WHERE key = ?1",
                params![s.key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional_dberr()?;
        match local {
            None => {
                tx.execute(
                    "INSERT OR IGNORE INTO user_settings (id, key, value, sync_hlc, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![s.id, s.key, s.value, s.sync_hlc, s.updated_at],
                )?;
            }
            Some((local_id, local_hlc)) if mode == RestoreMode::Merge => {
                let apply = match (&s.sync_hlc, &local_hlc) {
                    (Some(i), Some(l)) => i > l,
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if apply {
                    tx.execute(
                        "UPDATE user_settings SET value = ?1, sync_hlc = ?2, updated_at = ?3
                         WHERE id = ?4",
                        params![s.value, s.sync_hlc, s.updated_at, local_id],
                    )?;
                }
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn is_newer_updated_at(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id: &str,
    incoming_updated_at: &str,
) -> Result<bool, DbError> {
    let sql = format!("SELECT updated_at FROM {table} WHERE id = ?1");
    let local: Option<String> = tx
        .query_row(&sql, params![id], |r| r.get(0))
        .optional_dberr()?;
    Ok(match local {
        Some(l) => incoming_updated_at > l.as_str(),
        None => true,
    })
}

// ---------------------------------------------------------------------------
// Replace: clear all user-owned tables (children first for FK safety).
// ---------------------------------------------------------------------------

fn clear_library(tx: &rusqlite::Transaction<'_>) -> Result<(), DbError> {
    for table in [
        "book_authors",
        "isbns",
        "book_series",
        "book_shelves",
        "book_tags",
        "metadata_provenance",
        "reading_progress",
        "reading_sessions",
        "notes",
        "reviews",
        "files",
        "import_books",
        "import_logs",
        "sync_entity_hlc",
        "user_settings",
        "books",
        "authors",
        "series",
        "shelves",
        "tags",
        "works",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A remap from incoming entity ids to the local ids they resolved to.
struct IdMap {
    map: std::collections::HashMap<String, String>,
}

impl IdMap {
    fn new() -> Self {
        Self {
            map: std::collections::HashMap::new(),
        }
    }
    fn set(&mut self, from: &str, to: &str) {
        self.map.insert(from.to_string(), to.to_string());
    }
    /// Resolve an incoming id to its local id, defaulting to identity for ids
    /// that were never remapped (the common same-instance case).
    fn get(&self, from: &str) -> String {
        self.map
            .get(from)
            .cloned()
            .unwrap_or_else(|| from.to_string())
    }
}

fn query_vec<T, F>(conn: &rusqlite::Connection, sql: &str, f: F) -> Result<Vec<T>, DbError>
where
    F: Fn(&Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |r| f(r))?
        .collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(rows)
}

fn row_to_book(r: &Row<'_>) -> rusqlite::Result<BookRow> {
    Ok(BookRow {
        id: r.get(0)?,
        title: r.get(1)?,
        subtitle: r.get(2)?,
        description: r.get(3)?,
        page_count: r.get(4)?,
        pub_date: r.get(5)?,
        language: r.get(6)?,
        format: r.get(7)?,
        duration_minutes: r.get(8)?,
        cover_hash: r.get(9)?,
        work_id: r.get(10)?,
        status: r.get(11)?,
        rating: r.get(12)?,
        goodreads_id: r.get(13)?,
        calibre_id: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
        deleted_at: r.get(17)?,
        deleted_by_device: r.get(18)?,
    })
}

/// Convenience: map a rusqlite "no rows" into `Ok(None)` and other errors into
/// [`DbError`], so restore lookups stay terse.
trait OptionalDbErr<T> {
    fn optional_dberr(self) -> Result<Option<T>, DbError>;
}

impl<T> OptionalDbErr<T> for rusqlite::Result<T> {
    fn optional_dberr(self) -> Result<Option<T>, DbError> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use toku_core::{Author, Book, BookFormat, ContributorRole, ReadingStatus};

    #[test]
    fn export_then_replace_restore_roundtrips_a_book() {
        let src = Database::open_in_memory().unwrap();
        let repo = crate::BookRepository::new(&src);
        let mut book = Book::new("Dune");
        book.status = ReadingStatus::Read;
        book.format = BookFormat::Physical;
        book.rating = Some(9);
        repo.create_book(&book).unwrap();
        repo.add_book_author(
            &Author::new("Frank Herbert"),
            &book.id,
            ContributorRole::Author,
            0,
        )
        .unwrap();
        repo.add_isbn("9780441172719", &book.id).unwrap();

        let data = LibraryIo::new(&src).export_library().unwrap();
        assert_eq!(data.books.len(), 1);
        assert_eq!(data.isbns.len(), 1);

        let dst = Database::open_in_memory().unwrap();
        let res = LibraryIo::new(&dst)
            .restore_library(&data, RestoreMode::Replace)
            .unwrap();
        assert_eq!(res.books_inserted, 1);

        let data2 = LibraryIo::new(&dst).export_library().unwrap();
        assert_eq!(data.books, data2.books);
        assert_eq!(data.isbns, data2.isbns);
        assert_eq!(data.book_authors, data2.book_authors);
    }

    #[test]
    fn merge_is_idempotent() {
        let src = Database::open_in_memory().unwrap();
        let repo = crate::BookRepository::new(&src);
        let book = Book::new("Neuromancer");
        repo.create_book(&book).unwrap();
        let data = LibraryIo::new(&src).export_library().unwrap();

        let dst = Database::open_in_memory().unwrap();
        let io = LibraryIo::new(&dst);
        io.restore_library(&data, RestoreMode::Merge).unwrap();
        io.restore_library(&data, RestoreMode::Merge).unwrap();

        let n: i64 = dst
            .conn
            .query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
