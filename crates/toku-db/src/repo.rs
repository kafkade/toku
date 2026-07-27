use rusqlite::{OptionalExtension, params};
use toku_core::{
    Author, AuthorCount, Book, BookAuthor, BookSeries, ContributorRole, EntityType,
    FilterCondition, FilterExpr, FilterField, OpType, PaceRating, ReadingProgress, ReadingSession,
    ReadingStatus, Series, Shelf, SmartFilter, Tag, TagCount, TagType, Work,
};
use uuid::Uuid;

use crate::{Database, DbError, SyncRepository};

/// Book persistence operations.
pub struct BookRepository<'a> {
    db: &'a Database,
}

impl<'a> BookRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Run a mutation together with its sync-op emission atomically.
    ///
    /// A transaction is opened only when the connection is not already inside a
    /// caller-provided one — importers wrap whole batches in their own
    /// `BEGIN IMMEDIATE`, and SQLite forbids nested `BEGIN`. In the nested case
    /// the write and its op simply join the outer transaction. Either way the
    /// row change and the emitted op commit (or roll back) together.
    ///
    /// The closure receives a [`SyncRepository`] bound to the same connection;
    /// op emission is a no-op when no device identity is configured.
    fn with_sync_txn<T>(
        &self,
        f: impl FnOnce(&SyncRepository) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let sync = SyncRepository::new(self.db);
        if self.db.conn.is_autocommit() {
            let tx = self.db.conn.unchecked_transaction()?;
            let out = f(&sync)?;
            tx.commit()?;
            Ok(out)
        } else {
            f(&sync)
        }
    }

    /// Insert a new book. Returns the book's ID.
    pub fn create_book(&self, book: &Book) -> Result<(), DbError> {
        let search_text = build_search_text(
            &book.title,
            book.subtitle.as_deref(),
            book.description.as_deref(),
            &[],
        );
        self.with_sync_txn(|sync| {
            self.db.conn.execute(
                "INSERT INTO books (id, title, subtitle, description, page_count, pub_date,
                 language, format, duration_minutes, cover_hash, work_id, status, rating,
                 created_at, updated_at, search_text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    book.id.to_string(),
                    book.title,
                    book.subtitle,
                    book.description,
                    book.page_count,
                    book.pub_date,
                    book.language,
                    book.format.as_str(),
                    book.duration_minutes,
                    book.cover_hash,
                    book.work_id.map(|u| u.to_string()),
                    book.status.as_str(),
                    book.rating,
                    book.created_at.to_rfc3339(),
                    book.updated_at.to_rfc3339(),
                    search_text,
                ],
            )?;
            sync.emit_local_op(
                EntityType::Book,
                book.id,
                OpType::Create,
                Some(book_op_fields(book)),
            )
        })
    }

    /// Retrieve a book by its UUID.
    pub fn get_book(&self, id: &Uuid) -> Result<Book, DbError> {
        self.db
            .conn
            .query_row(
                "SELECT id, title, subtitle, description, page_count, pub_date,
                 language, format, duration_minutes, cover_hash, work_id, status,
                 rating, created_at, updated_at
                 FROM books WHERE id = ?1 AND deleted_at IS NULL",
                params![id.to_string()],
                |row| Ok(row_to_book(row)),
            )?
            .map_err(|e| DbError::Sqlite(rusqlite::Error::InvalidParameterName(e.to_string())))
    }

    /// List all books, ordered by title.
    pub fn list_books(&self) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status,
             rating, created_at, updated_at
             FROM books WHERE deleted_at IS NULL ORDER BY title COLLATE NOCASE",
        )?;

        let books = stmt
            .query_map([], |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    /// Full-text search across title, subtitle, description, and authors.
    pub fn search_books(&self, query: &str) -> Result<Vec<Book>, DbError> {
        self.search_books_filtered(query, None, None, None)
    }

    /// Full-text search with optional status, shelf, and tag filters.
    pub fn search_books_filtered(
        &self,
        query: &str,
        status: Option<&str>,
        shelf: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<Book>, DbError> {
        let mut sql = String::from(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM books_fts f
             JOIN books b ON b.rowid = f.rowid
             WHERE books_fts MATCH ?1 AND b.deleted_at IS NULL",
        );

        let mut param_index = 2u32;

        if status.is_some() {
            sql.push_str(&format!(" AND b.status = ?{param_index}"));
            param_index += 1;
        }

        if shelf.is_some() {
            sql.push_str(&format!(
                " AND b.id IN (SELECT bs.book_id FROM book_shelves bs
                 JOIN shelves s ON s.id = bs.shelf_id WHERE s.name = ?{param_index})"
            ));
            param_index += 1;
        }

        if tag.is_some() {
            sql.push_str(&format!(
                " AND b.id IN (SELECT bt.book_id FROM book_tags bt
                 JOIN tags t ON t.id = bt.tag_id WHERE t.name = ?{param_index})"
            ));
            // param_index += 1; // last param
        }

        sql.push_str(" ORDER BY rank");

        let mut stmt = self.db.conn.prepare(&sql)?;

        // Build dynamic params
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        values.push(Box::new(query.to_string()));
        if let Some(s) = status {
            values.push(Box::new(s.to_string()));
        }
        if let Some(s) = shelf {
            values.push(Box::new(s.to_string()));
        }
        if let Some(t) = tag {
            values.push(Box::new(t.to_string()));
        }

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();

        let books = stmt
            .query_map(params_ref.as_slice(), |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    /// Soft-delete a book by ID. Sets `deleted_at` instead of removing the row.
    pub fn delete_book(&self, id: &Uuid) -> Result<bool, DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.with_sync_txn(|sync| {
            let rows = self.db.conn.execute(
                "UPDATE books SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![now, id.to_string()],
            )?;
            if rows > 0 {
                sync.emit_local_op(EntityType::Book, *id, OpType::Delete, None)?;
            }
            Ok(rows > 0)
        })
    }

    /// Retrieve a book by ID, including soft-deleted books.
    pub fn get_book_including_deleted(&self, id: &Uuid) -> Result<Book, DbError> {
        self.db
            .conn
            .query_row(
                "SELECT id, title, subtitle, description, page_count, pub_date,
                 language, format, duration_minutes, cover_hash, work_id, status,
                 rating, created_at, updated_at
                 FROM books WHERE id = ?1",
                params![id.to_string()],
                |row| Ok(row_to_book(row)),
            )?
            .map_err(|e| DbError::Sqlite(rusqlite::Error::InvalidParameterName(e.to_string())))
    }

    /// Purge tombstoned books older than `retention_days`.
    /// Returns the number of permanently deleted rows.
    ///
    /// Only call this when all synced devices have had a chance to pull the
    /// delete ops — premature purging can cause deleted books to reappear on
    /// stale devices.
    pub fn purge_tombstones(&self, retention_days: i64) -> Result<usize, DbError> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days)).to_rfc3339();
        let rows = self.db.conn.execute(
            "DELETE FROM books WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
            params![cutoff],
        )?;
        Ok(rows)
    }

    /// Recompute and store the `search_text` column for a book, including author names.
    pub fn update_search_text(&self, book_id: &Uuid) -> Result<(), DbError> {
        let book = self.get_book(book_id)?;
        let authors = self.get_book_authors(book_id)?;
        let author_names: Vec<&str> = authors.iter().map(|(a, _)| a.name.as_str()).collect();
        let search_text = build_search_text(
            &book.title,
            book.subtitle.as_deref(),
            book.description.as_deref(),
            &author_names,
        );
        self.db.conn.execute(
            "UPDATE books SET search_text = ?1 WHERE id = ?2",
            params![search_text, book_id.to_string()],
        )?;
        Ok(())
    }

    /// Add an author and link them to a book.
    pub fn add_book_author(
        &self,
        author: &Author,
        book_id: &Uuid,
        role: ContributorRole,
        position: i32,
    ) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT OR IGNORE INTO authors (id, name, sort_name) VALUES (?1, ?2, ?3)",
            params![author.id.to_string(), author.name, author.sort_name],
        )?;

        self.db.conn.execute(
            "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                book_id.to_string(),
                author.id.to_string(),
                role.as_str(),
                position,
            ],
        )?;

        self.update_search_text(book_id)?;

        Ok(())
    }

    /// Get authors for a book.
    pub fn get_book_authors(&self, book_id: &Uuid) -> Result<Vec<(Author, BookAuthor)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT a.id, a.name, a.sort_name, ba.role, ba.position
             FROM book_authors ba
             JOIN authors a ON a.id = ba.author_id
             WHERE ba.book_id = ?1
             ORDER BY ba.position",
        )?;

        let results = stmt
            .query_map(params![book_id.to_string()], |row| {
                let author_id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let sort_name: Option<String> = row.get(2)?;
                let role_str: String = row.get(3)?;
                let position: i32 = row.get(4)?;

                let author = Author {
                    id: Uuid::parse_str(&author_id).unwrap_or_default(),
                    name,
                    sort_name,
                };

                let role = role_str.parse().unwrap_or(ContributorRole::Author);

                let book_author = BookAuthor {
                    book_id: *book_id,
                    author_id: author.id,
                    role,
                    position,
                };

                Ok((author, book_author))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Get all series a book belongs to, with its position in each.
    /// Ordered by series name for deterministic results.
    pub fn get_book_series(&self, book_id: &Uuid) -> Result<Vec<(Series, BookSeries)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT s.id, s.name, s.total_books, bs.position
             FROM book_series bs
             JOIN series s ON s.id = bs.series_id
             WHERE bs.book_id = ?1
             ORDER BY s.name",
        )?;

        let results = stmt
            .query_map(params![book_id.to_string()], |row| {
                let series_id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let total_books: Option<i32> = row.get(2)?;
                let position: Option<String> = row.get(3)?;

                let sid = Uuid::parse_str(&series_id).unwrap_or_default();
                let series = Series {
                    id: sid,
                    name,
                    total_books,
                };
                let book_series = BookSeries {
                    book_id: *book_id,
                    series_id: sid,
                    position,
                };
                Ok((series, book_series))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// List every series that has at least one (non-deleted) book, with the
    /// number of books in each. Ordered by series name.
    pub fn list_series(&self) -> Result<Vec<(Series, usize)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT s.id, s.name, s.total_books, COUNT(DISTINCT b.id) AS cnt
             FROM series s
             JOIN book_series bs ON bs.series_id = s.id
             JOIN books b ON b.id = bs.book_id AND b.deleted_at IS NULL
             GROUP BY s.id, s.name, s.total_books
             HAVING cnt > 0
             ORDER BY s.name COLLATE NOCASE",
        )?;

        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let total_books: Option<i32> = row.get(2)?;
                let count: i64 = row.get(3)?;
                Ok((
                    Series {
                        id: Uuid::parse_str(&id_str).unwrap_or_default(),
                        name,
                        total_books,
                    },
                    count as usize,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// List all (non-deleted) books in a series, matched case-insensitively by
    /// series name. Ordered by reading position when available, then title.
    pub fn list_books_in_series(&self, series_name: &str) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
                    b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id,
                    b.status, b.rating, b.created_at, b.updated_at
             FROM books b
             JOIN book_series bs ON bs.book_id = b.id
             JOIN series s ON s.id = bs.series_id
             WHERE LOWER(s.name) = LOWER(?1) AND b.deleted_at IS NULL
             ORDER BY CAST(bs.position AS REAL), b.title COLLATE NOCASE",
        )?;

        let books = stmt
            .query_map(params![series_name], |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    /// Store an ISBN linked to a book.
    pub fn add_isbn(&self, isbn: &str, book_id: &Uuid) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT OR IGNORE INTO isbns (isbn, book_id) VALUES (?1, ?2)",
            params![isbn, book_id.to_string()],
        )?;
        Ok(())
    }

    /// Get all ISBNs for a book.
    pub fn get_book_isbns(&self, book_id: &Uuid) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .db
            .conn
            .prepare("SELECT isbn FROM isbns WHERE book_id = ?1 ORDER BY isbn")?;

        let isbns = stmt
            .query_map(params![book_id.to_string()], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(isbns)
    }

    /// Find a book by ISBN.
    pub fn find_by_isbn(&self, isbn: &str) -> Result<Option<Book>, DbError> {
        let result = self.db.conn.query_row(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM isbns i
             JOIN books b ON b.id = i.book_id
             WHERE i.isbn = ?1 AND b.deleted_at IS NULL",
            params![isbn],
            |row| Ok(row_to_book(row)),
        );

        match result {
            Ok(Ok(book)) => Ok(Some(book)),
            Ok(Err(_)) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// Find a book by exact title (case-insensitive).
    pub fn find_book_by_title(&self, title: &str) -> Result<Option<Book>, DbError> {
        let result = self.db.conn.query_row(
            "SELECT id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status,
             rating, created_at, updated_at
             FROM books WHERE title = ?1 COLLATE NOCASE AND deleted_at IS NULL",
            params![title],
            |row| Ok(row_to_book(row)),
        );

        match result {
            Ok(Ok(book)) => Ok(Some(book)),
            Ok(Err(_)) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// Update a book's reading status.
    pub fn update_book_status(&self, id: &Uuid, status: ReadingStatus) -> Result<bool, DbError> {
        self.with_sync_txn(|sync| {
            let rows = self.db.conn.execute(
                "UPDATE books SET status = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
                params![
                    status.as_str(),
                    chrono::Utc::now().to_rfc3339(),
                    id.to_string(),
                ],
            )?;
            if rows > 0 {
                sync.emit_local_op(
                    EntityType::Book,
                    *id,
                    OpType::Update,
                    Some(serde_json::json!({ "status": status.as_str() })),
                )?;
            }
            Ok(rows > 0)
        })
    }

    /// Update a book's rating.
    pub fn update_book_rating(&self, id: &Uuid, rating: i32) -> Result<bool, DbError> {
        self.with_sync_txn(|sync| {
            let rows = self.db.conn.execute(
                "UPDATE books SET rating = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
                params![rating, chrono::Utc::now().to_rfc3339(), id.to_string(),],
            )?;
            if rows > 0 {
                sync.emit_local_op(
                    EntityType::Book,
                    *id,
                    OpType::Update,
                    Some(serde_json::json!({ "rating": rating })),
                )?;
            }
            Ok(rows > 0)
        })
    }

    /// Insert a new reading session.
    pub fn create_reading_session(&self, session: &ReadingSession) -> Result<(), DbError> {
        self.with_sync_txn(|sync| {
            self.db.conn.execute(
                "INSERT INTO reading_sessions (id, book_id, started_at, finished_at,
                 start_page, end_page, rating, notes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    session.id.to_string(),
                    session.book_id.to_string(),
                    session.started_at.to_rfc3339(),
                    session.finished_at.map(|d| d.to_rfc3339()),
                    session.start_page,
                    session.end_page,
                    session.rating,
                    session.notes,
                    session.created_at.to_rfc3339(),
                ],
            )?;
            sync.emit_local_op(
                EntityType::Session,
                session.id,
                OpType::Create,
                Some(serde_json::json!({
                    "book_id": session.book_id.to_string(),
                    "started_at": session.started_at.to_rfc3339(),
                    "finished_at": session.finished_at.map(|d| d.to_rfc3339()),
                    "start_page": session.start_page,
                    "end_page": session.end_page,
                    "rating": session.rating,
                    "notes": session.notes,
                })),
            )
        })
    }

    // --- Reading progress operations ---

    /// Insert a reading progress entry.
    pub fn log_progress(&self, progress: &ReadingProgress) -> Result<(), DbError> {
        self.with_sync_txn(|sync| {
            self.db.conn.execute(
                "INSERT INTO reading_progress (id, book_id, session_id, progress_type,
                 value, note, logged_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    progress.id.to_string(),
                    progress.book_id.to_string(),
                    progress.session_id.map(|u| u.to_string()),
                    progress.progress_type.as_str(),
                    progress.value,
                    progress.note,
                    progress.logged_at.to_rfc3339(),
                    progress.created_at.to_rfc3339(),
                ],
            )?;
            sync.emit_local_op(
                EntityType::Progress,
                progress.id,
                OpType::Create,
                Some(serde_json::json!({
                    "book_id": progress.book_id.to_string(),
                    "progress_type": progress.progress_type.as_str(),
                    "value": progress.value,
                    "session_id": progress.session_id.map(|u| u.to_string()),
                    "logged_at": progress.logged_at.to_rfc3339(),
                    "note": progress.note,
                })),
            )
        })
    }

    /// Get all progress entries for a book, ordered by `logged_at` DESC.
    pub fn get_reading_log(&self, book_id: &Uuid) -> Result<Vec<ReadingProgress>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, book_id, session_id, progress_type, value, note, logged_at, created_at
             FROM reading_progress
             WHERE book_id = ?1
             ORDER BY logged_at DESC",
        )?;

        let entries = stmt
            .query_map(params![book_id.to_string()], |row| {
                Ok(row_to_reading_progress(row))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Get the most recent progress entry for a book.
    pub fn get_latest_progress(&self, book_id: &Uuid) -> Result<Option<ReadingProgress>, DbError> {
        let result = self.db.conn.query_row(
            "SELECT id, book_id, session_id, progress_type, value, note, logged_at, created_at
             FROM reading_progress
             WHERE book_id = ?1
             ORDER BY logged_at DESC
             LIMIT 1",
            params![book_id.to_string()],
            |row| Ok(row_to_reading_progress(row)),
        );

        match result {
            Ok(Ok(progress)) => Ok(Some(progress)),
            Ok(Err(_)) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// Get the active (unfinished) reading session for a book.
    pub fn get_active_session(&self, book_id: &Uuid) -> Result<Option<ReadingSession>, DbError> {
        let result = self.db.conn.query_row(
            "SELECT id, book_id, started_at, finished_at, start_page, end_page,
             rating, notes, created_at
             FROM reading_sessions
             WHERE book_id = ?1 AND finished_at IS NULL
             ORDER BY started_at DESC
             LIMIT 1",
            params![book_id.to_string()],
            |row| Ok(row_to_reading_session(row)),
        );

        match result {
            Ok(Ok(session)) => Ok(Some(session)),
            Ok(Err(_)) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    // --- Shelf operations ---

    /// Create a new shelf with the given name.
    pub fn create_shelf(&self, name: &str) -> Result<Shelf, DbError> {
        let shelf = Shelf::new(name);
        self.db.conn.execute(
            "INSERT INTO shelves (id, name, is_smart, smart_filter, created_at) VALUES (?1, ?2, 0, NULL, ?3)",
            params![
                shelf.id.to_string(),
                shelf.name,
                shelf.created_at.to_rfc3339(),
            ],
        )?;
        Ok(shelf)
    }

    /// List all shelves ordered by name.
    pub fn list_shelves(&self) -> Result<Vec<Shelf>, DbError> {
        let mut stmt = self
            .db
            .conn
            .prepare("SELECT id, name, is_smart, smart_filter, created_at FROM shelves ORDER BY name COLLATE NOCASE")?;

        let shelves = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let is_smart: bool = row.get(2)?;
                let smart_filter: Option<String> = row.get(3)?;
                let created_str: String = row.get(4)?;
                Ok((id_str, name, is_smart, smart_filter, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, is_smart, smart_filter, created_str)| {
                Some(Shelf {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    is_smart,
                    smart_filter,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(shelves)
    }

    /// Add a book to a regular shelf, creating the shelf if it doesn't exist.
    /// Returns an error if the target shelf is a smart shelf.
    pub fn add_book_to_shelf(&self, book_id: &Uuid, shelf_name: &str) -> Result<(), DbError> {
        self.get_book(book_id)?;

        let row: Option<(String, bool)> = self
            .db
            .conn
            .query_row(
                "SELECT id, is_smart FROM shelves WHERE name = ?1",
                params![shelf_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let shelf_id = match row {
            Some((_, true)) => {
                return Err(DbError::InvalidOperation(format!(
                    "cannot manually add books to smart shelf '{shelf_name}'"
                )));
            }
            Some((id, false)) => id,
            None => {
                let shelf = self.create_shelf(shelf_name)?;
                shelf.id.to_string()
            }
        };

        self.db.conn.execute(
            "INSERT OR IGNORE INTO book_shelves (book_id, shelf_id) VALUES (?1, ?2)",
            params![book_id.to_string(), shelf_id],
        )?;

        Ok(())
    }

    /// Remove a book from a shelf.
    pub fn remove_book_from_shelf(&self, book_id: &Uuid, shelf_name: &str) -> Result<(), DbError> {
        let rows = self.db.conn.execute(
            "DELETE FROM book_shelves WHERE book_id = ?1
             AND shelf_id = (SELECT id FROM shelves WHERE name = ?2)",
            params![book_id.to_string(), shelf_name],
        )?;

        if rows == 0 {
            return Err(DbError::NotFound(format!(
                "book not on shelf '{shelf_name}'"
            )));
        }

        Ok(())
    }

    /// Get all shelves a book belongs to.
    pub fn get_book_shelves(&self, book_id: &Uuid) -> Result<Vec<Shelf>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT s.id, s.name, s.is_smart, s.smart_filter, s.created_at
             FROM shelves s
             JOIN book_shelves bs ON bs.shelf_id = s.id
             WHERE bs.book_id = ?1
             ORDER BY s.name COLLATE NOCASE",
        )?;

        let shelves = stmt
            .query_map(params![book_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let is_smart: bool = row.get(2)?;
                let smart_filter: Option<String> = row.get(3)?;
                let created_str: String = row.get(4)?;
                Ok((id_str, name, is_smart, smart_filter, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, is_smart, smart_filter, created_str)| {
                Some(Shelf {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    is_smart,
                    smart_filter,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(shelves)
    }

    /// List all books in a shelf. For smart shelves, evaluates the filter dynamically.
    pub fn list_books_in_shelf(&self, shelf_name: &str) -> Result<Vec<Book>, DbError> {
        // Check if this is a smart shelf
        let row: Option<(bool, Option<String>)> = self
            .db
            .conn
            .query_row(
                "SELECT is_smart, smart_filter FROM shelves WHERE name = ?1",
                params![shelf_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        match row {
            Some((true, Some(filter_json))) => {
                let filter =
                    SmartFilter::from_json(&filter_json).map_err(|e| DbError::Io(e.to_string()))?;
                self.evaluate_smart_filter(&filter)
            }
            Some((true, None)) => Ok(Vec::new()),
            Some((false, _)) | None => {
                // Regular shelf or not found — use book_shelves join
                let mut stmt = self.db.conn.prepare(
                    "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
                     b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
                     b.rating, b.created_at, b.updated_at
                     FROM books b
                     JOIN book_shelves bs ON bs.book_id = b.id
                     JOIN shelves s ON s.id = bs.shelf_id
                     WHERE s.name = ?1 AND b.deleted_at IS NULL
                     ORDER BY b.title COLLATE NOCASE",
                )?;

                let books = stmt
                    .query_map(params![shelf_name], |row| Ok(row_to_book(row)))?
                    .filter_map(|r| r.ok())
                    .filter_map(|r| r.ok())
                    .collect();

                Ok(books)
            }
        }
    }

    /// Create a smart shelf with a filter expression.
    pub fn create_smart_shelf(&self, name: &str, filter: &SmartFilter) -> Result<Shelf, DbError> {
        let filter_json = filter.to_json().map_err(|e| DbError::Io(e.to_string()))?;
        let shelf = Shelf::new_smart(name, filter_json.clone());
        self.db.conn.execute(
            "INSERT INTO shelves (id, name, is_smart, smart_filter, created_at) VALUES (?1, ?2, 1, ?3, ?4)",
            params![
                shelf.id.to_string(),
                shelf.name,
                filter_json,
                shelf.created_at.to_rfc3339(),
            ],
        )?;
        Ok(shelf)
    }

    /// Get a shelf by name.
    pub fn get_shelf_by_name(&self, name: &str) -> Result<Option<Shelf>, DbError> {
        self.db
            .conn
            .query_row(
                "SELECT id, name, is_smart, smart_filter, created_at FROM shelves WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |row| {
                    let id_str: String = row.get(0)?;
                    let name: String = row.get(1)?;
                    let is_smart: bool = row.get(2)?;
                    let smart_filter: Option<String> = row.get(3)?;
                    let created_str: String = row.get(4)?;
                    Ok((id_str, name, is_smart, smart_filter, created_str))
                },
            )
            .optional()?
            .and_then(|(id_str, name, is_smart, smart_filter, created_str)| {
                Some(Shelf {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    is_smart,
                    smart_filter,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .map_or_else(|| Ok(None), |s| Ok(Some(s)))
    }

    /// Delete a shelf by name.
    pub fn delete_shelf(&self, name: &str) -> Result<bool, DbError> {
        let rows = self
            .db
            .conn
            .execute("DELETE FROM shelves WHERE name = ?1", params![name])?;
        Ok(rows > 0)
    }

    /// Evaluate a smart filter and return matching books.
    pub fn evaluate_smart_filter(&self, filter: &SmartFilter) -> Result<Vec<Book>, DbError> {
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1u32;
        let where_clause = build_filter_sql(&filter.expression, &mut params, &mut param_idx);

        let sql = format!(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM books b WHERE b.deleted_at IS NULL AND ({where_clause})
             ORDER BY b.title COLLATE NOCASE"
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();

        let books = stmt
            .query_map(params_ref.as_slice(), |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    // --- Tag operations ---

    /// Create a tag (UPSERT — returns existing tag if name matches case-insensitively).
    pub fn create_tag(&self, name: &str) -> Result<Tag, DbError> {
        self.create_typed_tag(name, TagType::General)
    }

    /// Create or get a tag with a specific type. Tags are unique by (name, tag_type).
    pub fn create_typed_tag(&self, name: &str, tag_type: TagType) -> Result<Tag, DbError> {
        match self.db.conn.query_row(
            "SELECT id, name, tag_type, created_at FROM tags WHERE name = ?1 AND tag_type = ?2",
            params![name, tag_type.as_str()],
            |row| {
                let id_str: String = row.get(0)?;
                let tag_name: String = row.get(1)?;
                let type_str: String = row.get(2)?;
                let created_str: String = row.get(3)?;
                Ok((id_str, tag_name, type_str, created_str))
            },
        ) {
            Ok((id_str, tag_name, type_str, created_str)) => {
                let tag = Tag {
                    id: Uuid::parse_str(&id_str).map_err(|e| DbError::Io(e.to_string()))?,
                    name: tag_name,
                    tag_type: type_str.parse().unwrap_or(TagType::General),
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .map_err(|e| DbError::Io(e.to_string()))?
                        .with_timezone(&chrono::Utc),
                };
                Ok(tag)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let tag = Tag::with_type(name, tag_type);
                self.db.conn.execute(
                    "INSERT INTO tags (id, name, tag_type, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        tag.id.to_string(),
                        tag.name,
                        tag.tag_type.as_str(),
                        tag.created_at.to_rfc3339(),
                    ],
                )?;
                Ok(tag)
            }
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// Add a general tag to a book, creating the tag if it doesn't exist.
    pub fn add_tag_to_book(&self, book_id: &Uuid, tag_name: &str) -> Result<(), DbError> {
        self.add_typed_tag_to_book(book_id, tag_name, TagType::General)
    }

    /// Add a typed tag to a book, creating the tag if it doesn't exist.
    pub fn add_typed_tag_to_book(
        &self,
        book_id: &Uuid,
        tag_name: &str,
        tag_type: TagType,
    ) -> Result<(), DbError> {
        self.get_book(book_id)?;
        let tag = self.create_typed_tag(tag_name, tag_type)?;

        self.with_sync_txn(|sync| {
            let rows = self.db.conn.execute(
                "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
                params![book_id.to_string(), tag.id.to_string()],
            )?;
            if rows > 0 {
                sync.emit_local_op(
                    EntityType::Tag,
                    *book_id,
                    OpType::Create,
                    Some(serde_json::json!({
                        "tag_name": tag.name,
                        "tag_type": tag_type.as_str(),
                    })),
                )?;
            }
            Ok(())
        })
    }

    /// Set the pace rating for a book. Removes any existing pace tag first.
    pub fn set_book_pace(&self, book_id: &Uuid, pace: PaceRating) -> Result<(), DbError> {
        self.get_book(book_id)?;

        self.with_sync_txn(|sync| {
            // Capture the names of the pace tags currently on the book so the
            // removal can emit a Tag Delete op for each (merge matches by name).
            let existing: Vec<String> = {
                let mut stmt = self.db.conn.prepare(
                    "SELECT t.name FROM tags t
                     JOIN book_tags bt ON bt.tag_id = t.id
                     WHERE bt.book_id = ?1 AND t.tag_type = 'pace'",
                )?;
                let names =
                    stmt.query_map(params![book_id.to_string()], |row| row.get::<_, String>(0))?;
                names.filter_map(|r| r.ok()).collect()
            };

            let removed = self.db.conn.execute(
                "DELETE FROM book_tags WHERE book_id = ?1
                 AND tag_id IN (SELECT id FROM tags WHERE tag_type = 'pace')",
                params![book_id.to_string()],
            )?;
            if removed > 0 {
                for name in &existing {
                    sync.emit_local_op(
                        EntityType::Tag,
                        *book_id,
                        OpType::Delete,
                        Some(serde_json::json!({
                            "tag_name": name,
                            "tag_type": TagType::Pace.as_str(),
                        })),
                    )?;
                }
            }

            // Add the new pace tag in the same transaction.
            let tag = self.create_typed_tag(pace.as_str(), TagType::Pace)?;
            let added = self.db.conn.execute(
                "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
                params![book_id.to_string(), tag.id.to_string()],
            )?;
            if added > 0 {
                sync.emit_local_op(
                    EntityType::Tag,
                    *book_id,
                    OpType::Create,
                    Some(serde_json::json!({
                        "tag_name": tag.name,
                        "tag_type": TagType::Pace.as_str(),
                    })),
                )?;
            }
            Ok(())
        })
    }

    /// Remove a general tag from a book.
    pub fn remove_tag_from_book(&self, book_id: &Uuid, tag_name: &str) -> Result<(), DbError> {
        self.remove_typed_tag_from_book(book_id, tag_name, TagType::General)
    }

    /// Remove a typed tag from a book.
    pub fn remove_typed_tag_from_book(
        &self,
        book_id: &Uuid,
        tag_name: &str,
        tag_type: TagType,
    ) -> Result<(), DbError> {
        self.with_sync_txn(|sync| {
            let rows = self.db.conn.execute(
                "DELETE FROM book_tags WHERE book_id = ?1
                 AND tag_id IN (SELECT id FROM tags WHERE name = ?2 AND tag_type = ?3)",
                params![book_id.to_string(), tag_name, tag_type.as_str()],
            )?;

            if rows == 0 {
                return Err(DbError::NotFound(format!(
                    "tag '{tag_name}' ({tag_type}) not on this book"
                )));
            }

            sync.emit_local_op(
                EntityType::Tag,
                *book_id,
                OpType::Delete,
                Some(serde_json::json!({
                    "tag_name": tag_name,
                    "tag_type": tag_type.as_str(),
                })),
            )?;

            Ok(())
        })
    }

    /// Get all tags for a book.
    pub fn get_book_tags(&self, book_id: &Uuid) -> Result<Vec<Tag>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.tag_type, t.created_at
             FROM tags t
             JOIN book_tags bt ON bt.tag_id = t.id
             WHERE bt.book_id = ?1
             ORDER BY t.tag_type, t.name COLLATE NOCASE",
        )?;

        let tags = stmt
            .query_map(params![book_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let type_str: String = row.get(2)?;
                let created_str: String = row.get(3)?;
                Ok((id_str, name, type_str, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, type_str, created_str)| {
                Some(Tag {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    tag_type: type_str.parse().unwrap_or(TagType::General),
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(tags)
    }

    /// Get tags of a specific type for a book.
    pub fn get_book_tags_by_type(
        &self,
        book_id: &Uuid,
        tag_type: TagType,
    ) -> Result<Vec<Tag>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.tag_type, t.created_at
             FROM tags t
             JOIN book_tags bt ON bt.tag_id = t.id
             WHERE bt.book_id = ?1 AND t.tag_type = ?2
             ORDER BY t.name COLLATE NOCASE",
        )?;

        let tags = stmt
            .query_map(params![book_id.to_string(), tag_type.as_str()], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let type_str: String = row.get(2)?;
                let created_str: String = row.get(3)?;
                Ok((id_str, name, type_str, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, type_str, created_str)| {
                Some(Tag {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    tag_type: type_str.parse().unwrap_or(TagType::General),
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(tags)
    }

    /// List all books with a given general tag.
    pub fn list_books_by_tag(&self, tag_name: &str) -> Result<Vec<Book>, DbError> {
        self.list_books_by_typed_tag(tag_name, TagType::General)
    }

    /// List all books with a given typed tag.
    pub fn list_books_by_typed_tag(
        &self,
        tag_name: &str,
        tag_type: TagType,
    ) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM books b
             JOIN book_tags bt ON bt.book_id = b.id
             JOIN tags t ON t.id = bt.tag_id
             WHERE t.name = ?1 AND t.tag_type = ?2 AND b.deleted_at IS NULL
             ORDER BY b.title COLLATE NOCASE",
        )?;

        let books = stmt
            .query_map(params![tag_name, tag_type.as_str()], |row| {
                Ok(row_to_book(row))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    /// List all tags with book counts (excluding soft-deleted books).
    pub fn list_tags_with_counts(&self) -> Result<Vec<(Tag, i64)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.tag_type, t.created_at, COUNT(b.id) as book_count
             FROM tags t
             LEFT JOIN book_tags bt ON bt.tag_id = t.id
             LEFT JOIN books b ON b.id = bt.book_id AND b.deleted_at IS NULL
             GROUP BY t.id
             ORDER BY t.tag_type, t.name COLLATE NOCASE",
        )?;

        let tags = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let type_str: String = row.get(2)?;
                let created_str: String = row.get(3)?;
                let count: i64 = row.get(4)?;
                Ok((id_str, name, type_str, created_str, count))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, type_str, created_str, count)| {
                Some((
                    Tag {
                        id: Uuid::parse_str(&id_str).ok()?,
                        name,
                        tag_type: type_str.parse().unwrap_or(TagType::General),
                        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                            .ok()?
                            .with_timezone(&chrono::Utc),
                    },
                    count,
                ))
            })
            .collect();

        Ok(tags)
    }

    /// List tags of a specific type with book counts (excluding soft-deleted books).
    pub fn list_tags_with_counts_by_type(
        &self,
        tag_type: TagType,
    ) -> Result<Vec<(Tag, i64)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.tag_type, t.created_at, COUNT(b.id) as book_count
             FROM tags t
             LEFT JOIN book_tags bt ON bt.tag_id = t.id
             LEFT JOIN books b ON b.id = bt.book_id AND b.deleted_at IS NULL
             WHERE t.tag_type = ?1
             GROUP BY t.id
             ORDER BY t.name COLLATE NOCASE",
        )?;

        let tags = stmt
            .query_map(params![tag_type.as_str()], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let type_str: String = row.get(2)?;
                let created_str: String = row.get(3)?;
                let count: i64 = row.get(4)?;
                Ok((id_str, name, type_str, created_str, count))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, type_str, created_str, count)| {
                Some((
                    Tag {
                        id: Uuid::parse_str(&id_str).ok()?,
                        name,
                        tag_type: type_str.parse().unwrap_or(TagType::General),
                        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                            .ok()?
                            .with_timezone(&chrono::Utc),
                    },
                    count,
                ))
            })
            .collect();

        Ok(tags)
    }

    // --- Stats operations ---

    /// List all reading sessions.
    pub fn list_reading_sessions(&self) -> Result<Vec<ReadingSession>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, book_id, started_at, finished_at, start_page, end_page,
             rating, notes, created_at
             FROM reading_sessions
             ORDER BY started_at DESC",
        )?;

        let sessions = stmt
            .query_map([], |row| Ok(row_to_reading_session(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }

    /// List reading sessions finished in a given year.
    pub fn list_reading_sessions_in_year(&self, year: i32) -> Result<Vec<ReadingSession>, DbError> {
        let start = format!("{year}-01-01T00:00:00+00:00");
        let end = format!("{}-01-01T00:00:00+00:00", year + 1);

        let mut stmt = self.db.conn.prepare(
            "SELECT id, book_id, started_at, finished_at, start_page, end_page,
             rating, notes, created_at
             FROM reading_sessions
             WHERE finished_at IS NOT NULL
               AND finished_at >= ?1
               AND finished_at < ?2
             ORDER BY finished_at DESC",
        )?;

        let sessions = stmt
            .query_map(params![start, end], |row| Ok(row_to_reading_session(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }

    /// Count books grouped by reading status.
    pub fn count_books_by_status(
        &self,
    ) -> Result<std::collections::HashMap<String, usize>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT status, COUNT(*) FROM books WHERE deleted_at IS NULL GROUP BY status",
        )?;

        let mut map = std::collections::HashMap::new();
        let rows = stmt.query_map([], |row| {
            let status: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((status, count as usize))
        })?;

        for row in rows.flatten() {
            map.insert(row.0, row.1);
        }

        Ok(map)
    }

    /// Get books with status=Reading, their latest progress, and authors.
    #[allow(clippy::type_complexity)]
    pub fn get_currently_reading_details(
        &self,
    ) -> Result<Vec<(Book, Option<ReadingProgress>, Vec<(Author, BookAuthor)>)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status,
             rating, created_at, updated_at
             FROM books WHERE status = 'reading' AND deleted_at IS NULL
             ORDER BY updated_at DESC",
        )?;

        let books: Vec<Book> = stmt
            .query_map([], |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        let mut results = Vec::new();
        for book in books {
            let progress = self.get_latest_progress(&book.id)?;
            let authors = self.get_book_authors(&book.id)?;
            results.push((book, progress, authors));
        }

        Ok(results)
    }

    /// Get mood tag names for each book in a list of IDs.
    /// Returns a map of book_id string → list of mood tag names.
    pub fn get_mood_tags_for_books(
        &self,
        book_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, DbError> {
        if book_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let placeholders: String = book_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT bt.book_id, t.name
             FROM book_tags bt
             JOIN tags t ON t.id = bt.tag_id
             WHERE t.tag_type = 'mood' AND bt.book_id IN ({placeholders})
             ORDER BY bt.book_id, t.name"
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = book_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();

        let mut result: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

        let rows = stmt.query_map(params.as_slice(), |row| {
            let book_id: String = row.get(0)?;
            let mood: String = row.get(1)?;
            Ok((book_id, mood))
        })?;

        for row in rows.flatten() {
            result.entry(row.0).or_default().push(row.1);
        }

        Ok(result)
    }

    /// Tag names with book counts, sorted by count descending.
    pub fn list_tag_counts(&self) -> Result<Vec<TagCount>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.name, COUNT(bt.book_id) as cnt
             FROM tags t
             JOIN book_tags bt ON bt.tag_id = t.id
             GROUP BY t.name
             ORDER BY cnt DESC, t.name COLLATE NOCASE",
        )?;

        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok(TagCount {
                    name,
                    count: count as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Tag counts scoped to a specific set of book IDs.
    pub fn list_tag_counts_for_books(&self, book_ids: &[String]) -> Result<Vec<TagCount>, DbError> {
        if book_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = book_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT t.name, COUNT(bt.book_id) as cnt
             FROM tags t
             JOIN book_tags bt ON bt.tag_id = t.id
             WHERE bt.book_id IN ({placeholders})
             GROUP BY t.name
             ORDER BY cnt DESC, t.name COLLATE NOCASE"
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = book_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();

        let rows = stmt
            .query_map(params.as_slice(), |row| {
                let name: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok(TagCount {
                    name,
                    count: count as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Author names with book counts (role = author), sorted by count descending.
    pub fn list_author_book_counts(&self) -> Result<Vec<AuthorCount>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT a.name, COUNT(DISTINCT ba.book_id) as cnt
             FROM authors a
             JOIN book_authors ba ON ba.author_id = a.id
             WHERE ba.role = 'author'
             GROUP BY a.name
             ORDER BY cnt DESC, a.name COLLATE NOCASE",
        )?;

        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok(AuthorCount {
                    name,
                    count: count as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Author book counts scoped to specific book IDs.
    pub fn list_author_book_counts_for_books(
        &self,
        book_ids: &[String],
    ) -> Result<Vec<AuthorCount>, DbError> {
        if book_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = book_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT a.name, COUNT(DISTINCT ba.book_id) as cnt
             FROM authors a
             JOIN book_authors ba ON ba.author_id = a.id
             WHERE ba.role = 'author' AND ba.book_id IN ({placeholders})
             GROUP BY a.name
             ORDER BY cnt DESC, a.name COLLATE NOCASE"
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = book_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();

        let rows = stmt
            .query_map(params.as_slice(), |row| {
                let name: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok(AuthorCount {
                    name,
                    count: count as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Unique dates of reading activity (progress logs + session start/finish dates).
    pub fn list_activity_dates(&self) -> Result<Vec<chrono::NaiveDate>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT DISTINCT d FROM (
                 SELECT DATE(logged_at) AS d FROM reading_progress
                 UNION
                 SELECT DATE(started_at) AS d FROM reading_sessions
                 UNION
                 SELECT DATE(finished_at) AS d FROM reading_sessions
                     WHERE finished_at IS NOT NULL
             )
             ORDER BY d",
        )?;

        let dates = stmt
            .query_map([], |row| {
                let date_str: String = row.get(0)?;
                Ok(date_str)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .collect();

        Ok(dates)
    }

    /// Activity dates scoped to a specific year.
    pub fn list_activity_dates_in_year(
        &self,
        year: i32,
    ) -> Result<Vec<chrono::NaiveDate>, DbError> {
        let start = format!("{year}-01-01");
        let end = format!("{}-01-01", year + 1);

        let mut stmt = self.db.conn.prepare(
            "SELECT DISTINCT d FROM (
                 SELECT DATE(logged_at) AS d FROM reading_progress
                     WHERE DATE(logged_at) >= ?1 AND DATE(logged_at) < ?2
                 UNION
                 SELECT DATE(started_at) AS d FROM reading_sessions
                     WHERE DATE(started_at) >= ?1 AND DATE(started_at) < ?2
                 UNION
                 SELECT DATE(finished_at) AS d FROM reading_sessions
                     WHERE finished_at IS NOT NULL
                       AND DATE(finished_at) >= ?1 AND DATE(finished_at) < ?2
             )
             ORDER BY d",
        )?;

        let dates = stmt
            .query_map(params![start, end], |row| {
                let date_str: String = row.get(0)?;
                Ok(date_str)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .collect();

        Ok(dates)
    }

    /// Activity dates scoped to specific book IDs.
    pub fn list_activity_dates_for_books(
        &self,
        book_ids: &[String],
    ) -> Result<Vec<chrono::NaiveDate>, DbError> {
        if book_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = book_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT DISTINCT d FROM (
                 SELECT DATE(logged_at) AS d FROM reading_progress
                     WHERE book_id IN ({placeholders})
                 UNION
                 SELECT DATE(started_at) AS d FROM reading_sessions
                     WHERE book_id IN ({placeholders})
                 UNION
                 SELECT DATE(finished_at) AS d FROM reading_sessions
                     WHERE finished_at IS NOT NULL AND book_id IN ({placeholders})
             )
             ORDER BY d"
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        // Each subquery uses the same placeholders, so triple the params
        let mut all_params: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
        for _ in 0..3 {
            for id in book_ids {
                all_params.push(id as &dyn rusqlite::types::ToSql);
            }
        }

        let dates = stmt
            .query_map(all_params.as_slice(), |row| {
                let date_str: String = row.get(0)?;
                Ok(date_str)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .collect();

        Ok(dates)
    }

    /// List books by a specific author name (case-insensitive match).
    pub fn list_books_by_author_name(&self, name: &str) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT DISTINCT b.id, b.title, b.subtitle, b.description, b.page_count,
                    b.pub_date, b.language, b.format, b.duration_minutes, b.cover_hash,
                    b.work_id, b.status, b.rating, b.created_at, b.updated_at
             FROM books b
             JOIN book_authors ba ON ba.book_id = b.id
             JOIN authors a ON a.id = ba.author_id
             WHERE LOWER(a.name) = LOWER(?1) AND b.deleted_at IS NULL
             ORDER BY b.title COLLATE NOCASE",
        )?;

        let books = stmt
            .query_map(params![name], |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    /// List reading sessions for specific book IDs.
    pub fn list_sessions_for_books(
        &self,
        book_ids: &[String],
    ) -> Result<Vec<ReadingSession>, DbError> {
        if book_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = book_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, book_id, started_at, finished_at, start_page, end_page,
                    rating, notes, created_at
             FROM reading_sessions
             WHERE book_id IN ({placeholders})
             ORDER BY started_at DESC"
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = book_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();

        let sessions = stmt
            .query_map(params.as_slice(), |row| Ok(row_to_reading_session(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }

    /// List reading sessions for specific book IDs, filtered to a year.
    pub fn list_sessions_for_books_in_year(
        &self,
        book_ids: &[String],
        year: i32,
    ) -> Result<Vec<ReadingSession>, DbError> {
        if book_ids.is_empty() {
            return Ok(Vec::new());
        }

        let start = format!("{year}-01-01T00:00:00+00:00");
        let end = format!("{}-01-01T00:00:00+00:00", year + 1);
        let placeholders: String = book_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, book_id, started_at, finished_at, start_page, end_page,
                    rating, notes, created_at
             FROM reading_sessions
             WHERE book_id IN ({placeholders})
               AND finished_at IS NOT NULL
               AND finished_at >= ?{p1}
               AND finished_at < ?{p2}
             ORDER BY finished_at DESC",
            p1 = book_ids.len() + 1,
            p2 = book_ids.len() + 2,
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let mut all_params: Vec<&dyn rusqlite::types::ToSql> = book_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        all_params.push(&start as &dyn rusqlite::types::ToSql);
        all_params.push(&end as &dyn rusqlite::types::ToSql);

        let sessions = stmt
            .query_map(all_params.as_slice(), |row| Ok(row_to_reading_session(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }

    // ── Work methods ──────────────────────────────────────────────────

    /// Create a new work.
    pub fn create_work(&self, work: &Work) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT INTO works (id, title, original_language, first_published, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                work.id.to_string(),
                work.title,
                work.original_language,
                work.first_published,
                work.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Get a work by its UUID.
    pub fn get_work(&self, id: &Uuid) -> Result<Work, DbError> {
        self.db
            .conn
            .query_row(
                "SELECT id, title, original_language, first_published, created_at
                 FROM works WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    let id_str: String = row.get(0)?;
                    let created_str: String = row.get(4)?;
                    Ok((id_str, row.get(1)?, row.get(2)?, row.get(3)?, created_str))
                },
            )
            .map(
                |(id_str, title, original_language, first_published, created_str)| Work {
                    id: Uuid::parse_str(&id_str).expect("valid UUID"),
                    title,
                    original_language,
                    first_published,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .expect("valid datetime")
                        .with_timezone(&chrono::Utc),
                },
            )
            .map_err(DbError::from)
    }

    /// Find works whose title matches (case-insensitive LIKE).
    pub fn find_works_by_title(&self, title: &str) -> Result<Vec<Work>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, title, original_language, first_published, created_at
             FROM works WHERE title LIKE ?1 COLLATE NOCASE",
        )?;
        let works = stmt
            .query_map(params![format!("%{title}%")], |row| {
                let id_str: String = row.get(0)?;
                let created_str: String = row.get(4)?;
                Ok((id_str, row.get(1)?, row.get(2)?, row.get(3)?, created_str))
            })?
            .filter_map(|r| r.ok())
            .map(
                |(id_str, title, original_language, first_published, created_str)| Work {
                    id: Uuid::parse_str(&id_str).expect("valid UUID"),
                    title,
                    original_language,
                    first_published,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .expect("valid datetime")
                        .with_timezone(&chrono::Utc),
                },
            )
            .collect();
        Ok(works)
    }

    /// Link a book to a work by setting books.work_id.
    pub fn link_book_to_work(&self, book_id: &Uuid, work_id: &Uuid) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "UPDATE books SET work_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![work_id.to_string(), now, book_id.to_string()],
        )?;
        Ok(())
    }

    /// Unlink a book from its work (set work_id = NULL).
    pub fn unlink_book_from_work(&self, book_id: &Uuid) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "UPDATE books SET work_id = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, book_id.to_string()],
        )?;
        Ok(())
    }

    /// List all books belonging to a work (editions).
    pub fn get_work_editions(&self, work_id: &Uuid) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, title, subtitle, description, page_count, pub_date,
                    language, format, duration_minutes, cover_hash, work_id,
                    status, rating, created_at, updated_at
             FROM books WHERE work_id = ?1 AND deleted_at IS NULL",
        )?;
        let books = stmt
            .query_map(params![work_id.to_string()], |row| {
                row_to_book(row).map_err(rusqlite::Error::InvalidColumnName)
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(books)
    }

    /// Find groups of ungrouped books that share the same title and primary
    /// author (case-insensitive). Returns `(normalized_key, Vec<Book>)` pairs
    /// where each group has at least 2 members.
    pub fn auto_group_candidates(&self) -> Result<Vec<(String, Vec<Book>)>, DbError> {
        // Build a map: normalized(title + primary_author) → Vec<Book>
        let ungrouped = {
            let mut stmt = self.db.conn.prepare(
                "SELECT id, title, subtitle, description, page_count, pub_date,
                        language, format, duration_minutes, cover_hash, work_id,
                        status, rating, created_at, updated_at
                 FROM books WHERE work_id IS NULL AND deleted_at IS NULL",
            )?;
            let books: Vec<Book> = stmt
                .query_map([], |row| {
                    row_to_book(row).map_err(rusqlite::Error::InvalidColumnName)
                })?
                .filter_map(|r| r.ok())
                .collect();
            books
        };

        let mut groups: std::collections::HashMap<String, Vec<Book>> =
            std::collections::HashMap::new();
        for book in ungrouped {
            let authors = self.get_book_authors(&book.id)?;
            let primary_author = authors
                .iter()
                .find(|(_, ba)| ba.role == ContributorRole::Author)
                .map(|(a, _)| a.name.clone())
                .unwrap_or_default();
            let key = normalize_title_for_grouping(&book.title, &primary_author);
            groups.entry(key).or_default().push(book);
        }

        let mut result: Vec<(String, Vec<Book>)> = groups
            .into_iter()
            .filter(|(_, books)| books.len() >= 2)
            .collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }

    /// Merge book `removed_id` into `keep_id`. All dependent data
    /// (sessions, progress, ISBNs, tags, authors, series, provenance,
    /// import records, source IDs) is moved or merged, then the removed
    /// book is deleted.
    ///
    /// Fields on the kept book that are NULL are filled from the removed
    /// book. User-set provenance is never overwritten.
    pub fn merge_books(&self, keep_id: &Uuid, removed_id: &Uuid) -> Result<(), DbError> {
        let tx = self.db.conn.unchecked_transaction()?;

        // 1. Reassign reading_sessions
        tx.execute(
            "UPDATE reading_sessions SET book_id = ?1 WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;

        // 2. Reassign reading_progress
        tx.execute(
            "UPDATE reading_progress SET book_id = ?1 WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;

        // 3. Move ISBNs (skip duplicates)
        tx.execute(
            "UPDATE OR IGNORE isbns SET book_id = ?1 WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM isbns WHERE book_id = ?1",
            params![removed_id.to_string()],
        )?;

        // 4. Move tags (skip duplicates via OR IGNORE on composite PK)
        tx.execute(
            "INSERT OR IGNORE INTO book_tags (book_id, tag_id)
             SELECT ?1, tag_id FROM book_tags WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM book_tags WHERE book_id = ?1",
            params![removed_id.to_string()],
        )?;

        // 5. Move authors (skip duplicates via OR IGNORE)
        tx.execute(
            "INSERT OR IGNORE INTO book_authors (book_id, author_id, role)
             SELECT ?1, author_id, role FROM book_authors WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM book_authors WHERE book_id = ?1",
            params![removed_id.to_string()],
        )?;

        // 6. Move series memberships (skip duplicates)
        tx.execute(
            "INSERT OR IGNORE INTO book_series (book_id, series_id, position)
             SELECT ?1, series_id, position FROM book_series WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM book_series WHERE book_id = ?1",
            params![removed_id.to_string()],
        )?;

        // 7. Move provenance (only for fields not already tracked on keep)
        tx.execute(
            "INSERT OR IGNORE INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override)
             SELECT ?1, field_name, source, source_date, is_user_override
             FROM metadata_provenance WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM metadata_provenance WHERE book_id = ?1",
            params![removed_id.to_string()],
        )?;

        // 8. Move import records (skip duplicates)
        tx.execute(
            "INSERT OR IGNORE INTO import_books (import_id, book_id)
             SELECT import_id, ?1 FROM import_books WHERE book_id = ?2",
            params![keep_id.to_string(), removed_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM import_books WHERE book_id = ?1",
            params![removed_id.to_string()],
        )?;

        // 9. Fill NULL metadata fields on keep from removed
        let fill_fields = [
            "subtitle",
            "description",
            "page_count",
            "pub_date",
            "language",
            "duration_minutes",
            "cover_hash",
            "rating",
        ];
        for field in &fill_fields {
            let sql = format!(
                "UPDATE books SET {f} = (SELECT {f} FROM books WHERE id = ?2),
                 updated_at = ?3
                 WHERE id = ?1 AND {f} IS NULL
                 AND (SELECT {f} FROM books WHERE id = ?2) IS NOT NULL",
                f = field
            );
            tx.execute(
                &sql,
                params![
                    keep_id.to_string(),
                    removed_id.to_string(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )?;
        }

        // 10. Merge source IDs (goodreads_id, calibre_id): copy if keep is NULL
        tx.execute(
            "UPDATE books SET goodreads_id = (SELECT goodreads_id FROM books WHERE id = ?2),
             updated_at = ?3
             WHERE id = ?1 AND goodreads_id IS NULL
             AND (SELECT goodreads_id FROM books WHERE id = ?2) IS NOT NULL",
            params![
                keep_id.to_string(),
                removed_id.to_string(),
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        tx.execute(
            "UPDATE books SET calibre_id = (SELECT calibre_id FROM books WHERE id = ?2),
             updated_at = ?3
             WHERE id = ?1 AND calibre_id IS NULL
             AND (SELECT calibre_id FROM books WHERE id = ?2) IS NOT NULL",
            params![
                keep_id.to_string(),
                removed_id.to_string(),
                chrono::Utc::now().to_rfc3339()
            ],
        )?;

        // 11. If removed book had a work_id and keep doesn't, inherit it
        tx.execute(
            "UPDATE books SET work_id = (SELECT work_id FROM books WHERE id = ?2),
             updated_at = ?3
             WHERE id = ?1 AND work_id IS NULL
             AND (SELECT work_id FROM books WHERE id = ?2) IS NOT NULL",
            params![
                keep_id.to_string(),
                removed_id.to_string(),
                chrono::Utc::now().to_rfc3339()
            ],
        )?;

        // 12. Delete the removed book (ON DELETE CASCADE cleans up remaining refs)
        tx.execute(
            "DELETE FROM books WHERE id = ?1",
            params![removed_id.to_string()],
        )?;

        // Emit a sync op tombstoning the removed book, in the same transaction
        // so the merge and its op commit atomically. No-op without a device.
        SyncRepository::new(self.db).emit_local_op(
            EntityType::Book,
            *removed_id,
            OpType::Delete,
            None,
        )?;

        tx.commit()?;

        // 13. Recompute search_text outside the transaction
        self.update_search_text(keep_id)?;

        Ok(())
    }
}

/// Builds the sync-op field payload for a book Create op.
///
/// Mirrors the schema consumed by [`crate::merge`] (`BOOK_FIELDS`): enums are
/// serialized via their string form, numbers as integers. `work_id` is omitted
/// intentionally — Work rows are not synced yet, so shipping the reference would
/// dangle on the receiving device.
///
/// Exposed so importers (which insert books via raw SQL to also persist source
/// IDs) can emit the exact same Book Create op as [`BookRepository::create_book`].
pub fn book_op_fields(book: &Book) -> serde_json::Value {
    serde_json::json!({
        "title": book.title,
        "subtitle": book.subtitle,
        "description": book.description,
        "page_count": book.page_count,
        "pub_date": book.pub_date,
        "language": book.language,
        "format": book.format.as_str(),
        "duration_minutes": book.duration_minutes,
        "cover_hash": book.cover_hash,
        "status": book.status.as_str(),
        "rating": book.rating,
    })
}

/// Build the denormalized search_text from book fields and author names.
fn build_search_text(
    title: &str,
    subtitle: Option<&str>,
    description: Option<&str>,
    author_names: &[&str],
) -> String {
    let mut parts: Vec<&str> = vec![title];
    if let Some(s) = subtitle {
        parts.push(s);
    }
    if let Some(d) = description {
        parts.push(d);
    }
    for name in author_names {
        parts.push(name);
    }
    parts.join(" ")
}

/// Normalize a title + author string for grouping purposes.
/// Strips parentheticals, edition markers, extra whitespace; lowercases everything.
fn normalize_title_for_grouping(title: &str, author: &str) -> String {
    let mut s = title.to_lowercase();
    // Remove content in parentheses: "(Paperback)", "(2nd Edition)", etc.
    while let (Some(open), Some(close)) = (s.find('('), s.find(')')) {
        if open < close {
            s = format!("{}{}", &s[..open], &s[close + 1..]);
        } else {
            break;
        }
    }
    // Remove common edition markers
    for marker in &[
        "hardcover",
        "paperback",
        "mass market",
        "trade paperback",
        "kindle edition",
        "ebook",
        "audiobook",
        "audio cd",
        "board book",
        "library binding",
    ] {
        s = s.replace(marker, "");
    }
    // Collapse whitespace and trim
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let author_norm = author.to_lowercase().trim().to_string();
    if author_norm.is_empty() {
        s
    } else {
        format!("{s} | {author_norm}")
    }
}

fn row_to_book(row: &rusqlite::Row<'_>) -> Result<Book, String> {
    let id_str: String = row.get(0).map_err(|e| e.to_string())?;
    let format_str: String = row.get(7).map_err(|e| e.to_string())?;
    let status_str: String = row.get(11).map_err(|e| e.to_string())?;
    let created_str: String = row.get(13).map_err(|e| e.to_string())?;
    let updated_str: String = row.get(14).map_err(|e| e.to_string())?;
    let work_id_str: Option<String> = row.get(10).map_err(|e| e.to_string())?;

    Ok(Book {
        id: Uuid::parse_str(&id_str).map_err(|e| e.to_string())?,
        title: row.get(1).map_err(|e| e.to_string())?,
        subtitle: row.get(2).map_err(|e| e.to_string())?,
        description: row.get(3).map_err(|e| e.to_string())?,
        page_count: row.get(4).map_err(|e| e.to_string())?,
        pub_date: row.get(5).map_err(|e| e.to_string())?,
        language: row.get(6).map_err(|e| e.to_string())?,
        format: format_str
            .parse()
            .map_err(|e: toku_core::TokuError| e.to_string())?,
        duration_minutes: row.get(8).map_err(|e| e.to_string())?,
        cover_hash: row.get(9).map_err(|e| e.to_string())?,
        work_id: work_id_str
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| e.to_string())?,
        status: status_str
            .parse()
            .map_err(|e: toku_core::TokuError| e.to_string())?,
        rating: row.get(12).map_err(|e| e.to_string())?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&chrono::Utc),
    })
}

fn row_to_reading_progress(row: &rusqlite::Row<'_>) -> Result<ReadingProgress, String> {
    let id_str: String = row.get(0).map_err(|e| e.to_string())?;
    let book_id_str: String = row.get(1).map_err(|e| e.to_string())?;
    let session_id_str: Option<String> = row.get(2).map_err(|e| e.to_string())?;
    let progress_type_str: String = row.get(3).map_err(|e| e.to_string())?;
    let logged_str: String = row.get(6).map_err(|e| e.to_string())?;
    let created_str: String = row.get(7).map_err(|e| e.to_string())?;

    Ok(ReadingProgress {
        id: Uuid::parse_str(&id_str).map_err(|e| e.to_string())?,
        book_id: Uuid::parse_str(&book_id_str).map_err(|e| e.to_string())?,
        session_id: session_id_str
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| e.to_string())?,
        progress_type: progress_type_str
            .parse()
            .map_err(|e: toku_core::TokuError| e.to_string())?,
        value: row.get(4).map_err(|e| e.to_string())?,
        note: row.get(5).map_err(|e| e.to_string())?,
        logged_at: chrono::DateTime::parse_from_rfc3339(&logged_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&chrono::Utc),
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&chrono::Utc),
    })
}

fn row_to_reading_session(row: &rusqlite::Row<'_>) -> Result<ReadingSession, String> {
    let id_str: String = row.get(0).map_err(|e| e.to_string())?;
    let book_id_str: String = row.get(1).map_err(|e| e.to_string())?;
    let started_str: String = row.get(2).map_err(|e| e.to_string())?;
    let finished_str: Option<String> = row.get(3).map_err(|e| e.to_string())?;
    let created_str: String = row.get(8).map_err(|e| e.to_string())?;

    Ok(ReadingSession {
        id: Uuid::parse_str(&id_str).map_err(|e| e.to_string())?,
        book_id: Uuid::parse_str(&book_id_str).map_err(|e| e.to_string())?,
        started_at: chrono::DateTime::parse_from_rfc3339(&started_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&chrono::Utc),
        finished_at: finished_str
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&chrono::Utc))
            })
            .transpose()
            .map_err(|e| e.to_string())?,
        start_page: row.get(4).map_err(|e| e.to_string())?,
        end_page: row.get(5).map_err(|e| e.to_string())?,
        rating: row.get(6).map_err(|e| e.to_string())?,
        notes: row.get(7).map_err(|e| e.to_string())?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&chrono::Utc),
    })
}

/// Escape LIKE wildcards in a value for safe parameterized LIKE queries.
fn escape_like_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Recursively build a SQL WHERE clause from a filter expression tree.
/// Appends parameterized values to `params` and increments `param_idx`.
fn build_filter_sql(
    expr: &FilterExpr,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    param_idx: &mut u32,
) -> String {
    match expr {
        FilterExpr::And(exprs) => {
            let clauses: Vec<String> = exprs
                .iter()
                .map(|e| {
                    let clause = build_filter_sql(e, params, param_idx);
                    if matches!(e, FilterExpr::Or(_)) {
                        format!("({clause})")
                    } else {
                        clause
                    }
                })
                .collect();
            clauses.join(" AND ")
        }
        FilterExpr::Or(exprs) => {
            let clauses: Vec<String> = exprs
                .iter()
                .map(|e| build_filter_sql(e, params, param_idx))
                .collect();
            clauses.join(" OR ")
        }
        FilterExpr::Condition(cond) => build_condition_sql(cond, params, param_idx),
    }
}

fn build_condition_sql(
    cond: &FilterCondition,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    param_idx: &mut u32,
) -> String {
    let idx = *param_idx;
    *param_idx += 1;

    match cond.field {
        FilterField::Status => {
            // Normalize underscores to hyphens (DSL accepts both)
            let normalized = cond.value.replace('_', "-");
            params.push(Box::new(normalized));
            format!("b.status = ?{idx}")
        }
        FilterField::Rating => {
            let val: i64 = cond.value.parse().unwrap_or(0);
            params.push(Box::new(val));
            format!("b.rating {} ?{idx}", cond.op.as_sql())
        }
        FilterField::Pages => {
            let val: i64 = cond.value.parse().unwrap_or(0);
            params.push(Box::new(val));
            format!("b.page_count {} ?{idx}", cond.op.as_sql())
        }
        FilterField::Format => {
            params.push(Box::new(cond.value.clone()));
            format!("b.format = ?{idx}")
        }
        FilterField::PubDate => {
            params.push(Box::new(cond.value.clone()));
            if cond.value.len() == 4 {
                // Year comparison: extract first 4 chars of pub_date
                format!("SUBSTR(b.pub_date, 1, 4) {} ?{idx}", cond.op.as_sql())
            } else {
                format!("b.pub_date {} ?{idx}", cond.op.as_sql())
            }
        }
        FilterField::DateAdded => {
            params.push(Box::new(cond.value.clone()));
            if cond.value.len() == 4 {
                format!("SUBSTR(b.created_at, 1, 4) {} ?{idx}", cond.op.as_sql())
            } else {
                format!("SUBSTR(b.created_at, 1, 10) {} ?{idx}", cond.op.as_sql())
            }
        }
        FilterField::Tag => {
            params.push(Box::new(cond.value.clone()));
            format!(
                "EXISTS (SELECT 1 FROM book_tags bt JOIN tags t ON t.id = bt.tag_id \
                 WHERE bt.book_id = b.id AND t.name = ?{idx} COLLATE NOCASE AND t.tag_type = 'general')"
            )
        }
        FilterField::Mood => {
            params.push(Box::new(cond.value.clone()));
            format!(
                "EXISTS (SELECT 1 FROM book_tags bt JOIN tags t ON t.id = bt.tag_id \
                 WHERE bt.book_id = b.id AND t.name = ?{idx} COLLATE NOCASE AND t.tag_type = 'mood')"
            )
        }
        FilterField::Pace => {
            params.push(Box::new(cond.value.clone()));
            format!(
                "EXISTS (SELECT 1 FROM book_tags bt JOIN tags t ON t.id = bt.tag_id \
                 WHERE bt.book_id = b.id AND t.name = ?{idx} COLLATE NOCASE AND t.tag_type = 'pace')"
            )
        }
        FilterField::Author => {
            let escaped = format!("%{}%", escape_like_value(&cond.value));
            params.push(Box::new(escaped));
            format!(
                "EXISTS (SELECT 1 FROM book_authors ba JOIN authors a ON a.id = ba.author_id \
                 WHERE ba.book_id = b.id AND a.name LIKE ?{idx} ESCAPE '\\' COLLATE NOCASE)"
            )
        }
        FilterField::Shelf => {
            params.push(Box::new(cond.value.clone()));
            format!(
                "EXISTS (SELECT 1 FROM book_shelves bs JOIN shelves s ON s.id = bs.shelf_id \
                 WHERE bs.book_id = b.id AND s.name = ?{idx} COLLATE NOCASE AND s.is_smart = 0)"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use toku_core::{ProgressType, ReadingStatus};

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn create_and_get_book() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let retrieved = repo.get_book(&book.id).unwrap();
        assert_eq!(retrieved.title, "Dune");
        assert_eq!(retrieved.status, ReadingStatus::WantToRead);
    }

    #[test]
    fn list_books_ordered_by_title() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Neuromancer");
        b1.id = Uuid::now_v7();
        let mut b2 = Book::new("Dune");
        b2.id = Uuid::now_v7();

        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();

        let books = repo.list_books().unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[1].title, "Neuromancer");
    }

    #[test]
    fn lists_series_and_books_in_series() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Dune Messiah");
        b1.id = Uuid::now_v7();
        let mut b2 = Book::new("Dune");
        b2.id = Uuid::now_v7();
        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();

        let series_id = Uuid::now_v7();
        db.conn
            .execute(
                "INSERT INTO series (id, name, total_books) VALUES (?1, ?2, ?3)",
                params![series_id.to_string(), "Dune", 2],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO book_series (book_id, series_id, position) VALUES (?1, ?2, ?3)",
                params![b1.id.to_string(), series_id.to_string(), "2"],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO book_series (book_id, series_id, position) VALUES (?1, ?2, ?3)",
                params![b2.id.to_string(), series_id.to_string(), "1"],
            )
            .unwrap();

        let series = repo.list_series().unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].0.name, "Dune");
        assert_eq!(series[0].1, 2);

        // Ordered by numeric position: Dune (1) before Dune Messiah (2).
        let books = repo.list_books_in_series("dune").unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Dune");
        assert_eq!(books[1].title, "Dune Messiah");
    }

    #[test]
    fn search_books_fts() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Dune");
        b1.description = Some("A science fiction masterpiece about desert planets".to_string());
        repo.create_book(&b1).unwrap();

        let mut b2 = Book::new("Neuromancer");
        b2.description = Some("Cyberpunk novel about hackers and AI".to_string());
        repo.create_book(&b2).unwrap();

        let results = repo.search_books("desert").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Dune");
    }

    #[test]
    fn delete_book() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("To Delete");
        repo.create_book(&book).unwrap();
        assert!(repo.delete_book(&book.id).unwrap());
        assert!(repo.list_books().unwrap().is_empty());
    }

    #[test]
    fn add_and_get_authors() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let author = Author::new("Frank Herbert");
        repo.add_book_author(&author, &book.id, ContributorRole::Author, 0)
            .unwrap();

        let authors = repo.get_book_authors(&book.id).unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].0.name, "Frank Herbert");
        assert_eq!(authors[0].1.role, ContributorRole::Author);
    }

    #[test]
    fn find_by_isbn() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();
        repo.add_isbn("9780441013593", &book.id).unwrap();

        let found = repo.find_by_isbn("9780441013593").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Dune");

        let not_found = repo.find_by_isbn("9780000000000").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn update_book_status() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();
        assert_eq!(
            repo.get_book(&book.id).unwrap().status,
            ReadingStatus::WantToRead
        );

        assert!(
            repo.update_book_status(&book.id, ReadingStatus::Reading)
                .unwrap()
        );
        assert_eq!(
            repo.get_book(&book.id).unwrap().status,
            ReadingStatus::Reading
        );

        // Non-existent ID returns false
        let fake_id = Uuid::now_v7();
        assert!(
            !repo
                .update_book_status(&fake_id, ReadingStatus::Reading)
                .unwrap()
        );
    }

    #[test]
    fn update_book_rating() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();
        assert!(book.rating.is_none());

        assert!(repo.update_book_rating(&book.id, 8).unwrap());
        assert_eq!(repo.get_book(&book.id).unwrap().rating, Some(8));
    }

    #[test]
    fn create_reading_session() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let session = toku_core::ReadingSession::new(book.id);
        repo.create_reading_session(&session).unwrap();

        // Verify it was inserted
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM reading_sessions WHERE book_id = ?1",
                params![book.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn find_book_by_title_case_insensitive() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        // Exact match
        let found = repo.find_book_by_title("Dune").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Dune");

        // Case-insensitive
        let found = repo.find_book_by_title("dune").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Dune");

        let found = repo.find_book_by_title("DUNE").unwrap();
        assert!(found.is_some());

        // No match
        let found = repo.find_book_by_title("Neuromancer").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn create_and_list_shelves() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        repo.create_shelf("Favorites").unwrap();
        repo.create_shelf("To Re-read").unwrap();

        let shelves = repo.list_shelves().unwrap();
        assert_eq!(shelves.len(), 2);
        assert_eq!(shelves[0].name, "Favorites");
        assert_eq!(shelves[1].name, "To Re-read");
    }

    #[test]
    fn add_and_remove_book_from_shelf() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        repo.add_book_to_shelf(&book.id, "Favorites").unwrap();

        let shelves = repo.get_book_shelves(&book.id).unwrap();
        assert_eq!(shelves.len(), 1);
        assert_eq!(shelves[0].name, "Favorites");

        let books = repo.list_books_in_shelf("Favorites").unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Dune");

        repo.remove_book_from_shelf(&book.id, "Favorites").unwrap();
        let shelves = repo.get_book_shelves(&book.id).unwrap();
        assert!(shelves.is_empty());
    }

    #[test]
    fn shelf_multiple_books() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Dune");
        let b2 = Book::new("Neuromancer");
        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();

        repo.add_book_to_shelf(&b1.id, "Sci-Fi").unwrap();
        repo.add_book_to_shelf(&b2.id, "Sci-Fi").unwrap();

        let books = repo.list_books_in_shelf("Sci-Fi").unwrap();
        assert_eq!(books.len(), 2);
    }

    #[test]
    fn create_tag_upsert() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let t1 = repo.create_tag("sci-fi").unwrap();
        let t2 = repo.create_tag("Sci-Fi").unwrap();
        assert_eq!(t1.id, t2.id);
    }

    #[test]
    fn add_and_remove_tag() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        repo.add_tag_to_book(&book.id, "sci-fi").unwrap();
        repo.add_tag_to_book(&book.id, "classic").unwrap();

        let tags = repo.get_book_tags(&book.id).unwrap();
        assert_eq!(tags.len(), 2);

        repo.remove_tag_from_book(&book.id, "sci-fi").unwrap();
        let tags = repo.get_book_tags(&book.id).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "classic");
    }

    #[test]
    fn list_books_by_tag_test() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Dune");
        let b2 = Book::new("Neuromancer");
        let b3 = Book::new("War and Peace");
        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();
        repo.create_book(&b3).unwrap();

        repo.add_tag_to_book(&b1.id, "sci-fi").unwrap();
        repo.add_tag_to_book(&b2.id, "sci-fi").unwrap();

        let results = repo.list_books_by_tag("sci-fi").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn list_tags_with_counts_test() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Dune");
        let b2 = Book::new("Neuromancer");
        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();

        repo.add_tag_to_book(&b1.id, "sci-fi").unwrap();
        repo.add_tag_to_book(&b2.id, "sci-fi").unwrap();
        repo.add_tag_to_book(&b1.id, "classic").unwrap();

        let tags = repo.list_tags_with_counts().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].0.name, "classic");
        assert_eq!(tags[0].1, 1);
        assert_eq!(tags[1].0.name, "sci-fi");
        assert_eq!(tags[1].1, 2);
    }

    #[test]
    fn book_delete_cascades_shelf_and_tag() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();
        repo.add_book_to_shelf(&book.id, "Favorites").unwrap();
        repo.add_tag_to_book(&book.id, "sci-fi").unwrap();

        repo.delete_book(&book.id).unwrap();

        let books_in_shelf = repo.list_books_in_shelf("Favorites").unwrap();
        assert!(books_in_shelf.is_empty());
        let books_with_tag = repo.list_books_by_tag("sci-fi").unwrap();
        assert!(books_with_tag.is_empty());
    }

    #[test]
    fn search_by_author_name() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let author = Author::new("Frank Herbert");
        repo.add_book_author(&author, &book.id, ContributorRole::Author, 0)
            .unwrap();

        let results = repo.search_books("Herbert").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Dune");
    }

    #[test]
    fn search_filtered_by_status() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Dune");
        b1.description = Some("desert planet adventure".to_string());
        repo.create_book(&b1).unwrap();
        repo.update_book_status(&b1.id, ReadingStatus::Reading)
            .unwrap();

        let mut b2 = Book::new("Dune Messiah");
        b2.description = Some("sequel on a desert world".to_string());
        repo.create_book(&b2).unwrap();

        // Both match "desert", but only b1 is "reading"
        let results = repo
            .search_books_filtered("desert", Some("reading"), None, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Dune");
    }

    #[test]
    fn search_filtered_by_shelf() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Dune");
        b1.description = Some("desert planet adventure".to_string());
        repo.create_book(&b1).unwrap();
        repo.add_book_to_shelf(&b1.id, "Favorites").unwrap();

        let mut b2 = Book::new("Dune Messiah");
        b2.description = Some("sequel on a desert world".to_string());
        repo.create_book(&b2).unwrap();

        let results = repo
            .search_books_filtered("desert", None, Some("Favorites"), None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Dune");
    }

    #[test]
    fn search_filtered_by_tag() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Dune");
        b1.description = Some("desert planet adventure".to_string());
        repo.create_book(&b1).unwrap();
        repo.add_tag_to_book(&b1.id, "classic").unwrap();

        let mut b2 = Book::new("Dune Messiah");
        b2.description = Some("sequel on a desert world".to_string());
        repo.create_book(&b2).unwrap();

        let results = repo
            .search_books_filtered("desert", None, None, Some("classic"))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Dune");
    }

    #[test]
    fn search_no_results() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let results = repo.search_books("xyzzyplugh").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn log_progress_and_get_reading_log() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let mut p1 = ReadingProgress::new(book.id, ProgressType::Page, 50);
        p1.note = Some("Getting interesting".to_string());
        repo.log_progress(&p1).unwrap();

        let p2 = ReadingProgress::new(book.id, ProgressType::Page, 100);
        repo.log_progress(&p2).unwrap();

        let log = repo.get_reading_log(&book.id).unwrap();
        assert_eq!(log.len(), 2);
        // DESC order — most recent first
        assert_eq!(log[0].value, 100);
        assert_eq!(log[1].value, 50);
        assert_eq!(log[1].note.as_deref(), Some("Getting interesting"));
        assert_eq!(log[0].progress_type, ProgressType::Page);
    }

    #[test]
    fn get_latest_progress_returns_most_recent() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        // No progress yet
        assert!(repo.get_latest_progress(&book.id).unwrap().is_none());

        let p1 = ReadingProgress::new(book.id, ProgressType::Page, 50);
        repo.log_progress(&p1).unwrap();

        let p2 = ReadingProgress::new(book.id, ProgressType::Page, 150);
        repo.log_progress(&p2).unwrap();

        let latest = repo.get_latest_progress(&book.id).unwrap().unwrap();
        assert_eq!(latest.value, 150);
    }

    #[test]
    fn get_active_session_returns_unfinished() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        // No sessions yet
        assert!(repo.get_active_session(&book.id).unwrap().is_none());

        let session = ReadingSession::new(book.id);
        repo.create_reading_session(&session).unwrap();

        let active = repo.get_active_session(&book.id).unwrap();
        assert!(active.is_some());
        assert_eq!(active.unwrap().id, session.id);
    }

    #[test]
    fn progress_with_session_link() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Dune");
        repo.create_book(&book).unwrap();

        let session = ReadingSession::new(book.id);
        repo.create_reading_session(&session).unwrap();

        let mut progress = ReadingProgress::new(book.id, ProgressType::Percent, 45);
        progress.session_id = Some(session.id);
        repo.log_progress(&progress).unwrap();

        let log = repo.get_reading_log(&book.id).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].session_id, Some(session.id));
        assert_eq!(log[0].progress_type, ProgressType::Percent);
    }

    // ── Work & merge tests ────────────────────────────────────────────

    #[test]
    fn create_and_get_work() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let work = Work::new("Dune");
        repo.create_work(&work).unwrap();

        let retrieved = repo.get_work(&work.id).unwrap();
        assert_eq!(retrieved.title, "Dune");
        assert!(retrieved.original_language.is_none());
    }

    #[test]
    fn find_works_by_title_case_insensitive() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let w1 = Work::new("Dune");
        let w2 = Work::new("The Left Hand of Darkness");
        repo.create_work(&w1).unwrap();
        repo.create_work(&w2).unwrap();

        let found = repo.find_works_by_title("dune").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Dune");

        let found = repo.find_works_by_title("DARKNESS").unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn link_and_unlink_book_to_work() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let work = Work::new("Dune");
        repo.create_work(&work).unwrap();

        let book = Book::new("Dune (Paperback)");
        repo.create_book(&book).unwrap();
        assert!(repo.get_book(&book.id).unwrap().work_id.is_none());

        repo.link_book_to_work(&book.id, &work.id).unwrap();
        assert_eq!(repo.get_book(&book.id).unwrap().work_id, Some(work.id));

        let editions = repo.get_work_editions(&work.id).unwrap();
        assert_eq!(editions.len(), 1);
        assert_eq!(editions[0].id, book.id);

        repo.unlink_book_from_work(&book.id).unwrap();
        assert!(repo.get_book(&book.id).unwrap().work_id.is_none());
    }

    #[test]
    fn merge_books_moves_sessions_and_progress() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let keep = Book::new("Dune");
        let remove = Book::new("Dune (Paperback)");
        repo.create_book(&keep).unwrap();
        repo.create_book(&remove).unwrap();

        // Add sessions and progress to the removed book
        let session = ReadingSession::new(remove.id);
        repo.create_reading_session(&session).unwrap();
        let progress = ReadingProgress::new(remove.id, ProgressType::Page, 50);
        repo.log_progress(&progress).unwrap();

        repo.merge_books(&keep.id, &remove.id).unwrap();

        // Sessions and progress moved to keep
        let sessions = repo.list_reading_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].book_id, keep.id);

        let log = repo.get_reading_log(&keep.id).unwrap();
        assert_eq!(log.len(), 1);

        // Removed book is gone
        assert!(repo.get_book(&remove.id).is_err());
    }

    #[test]
    fn merge_books_dedupes_tags_and_authors() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let keep = Book::new("Dune");
        let remove = Book::new("Dune (Kindle)");
        repo.create_book(&keep).unwrap();
        repo.create_book(&remove).unwrap();

        // Both have the same tag "sci-fi" and an exclusive tag each
        repo.add_tag_to_book(&keep.id, "sci-fi").unwrap();
        repo.add_tag_to_book(&keep.id, "classic").unwrap();
        repo.add_tag_to_book(&remove.id, "sci-fi").unwrap();
        repo.add_tag_to_book(&remove.id, "favorite").unwrap();

        // Both have the same author
        let frank = Author::new("Frank Herbert");
        repo.add_book_author(&frank, &keep.id, ContributorRole::Author, 0)
            .unwrap();
        repo.add_book_author(&frank, &remove.id, ContributorRole::Author, 0)
            .unwrap();

        repo.merge_books(&keep.id, &remove.id).unwrap();

        let tags = repo.get_book_tags(&keep.id).unwrap();
        assert_eq!(tags.len(), 3); // sci-fi, classic, favorite (deduped)

        let authors = repo.get_book_authors(&keep.id).unwrap();
        assert_eq!(authors.len(), 1); // Frank Herbert (deduped)
    }

    #[test]
    fn merge_books_fills_null_metadata() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let keep = Book::new("Dune");
        repo.create_book(&keep).unwrap();

        let mut remove = Book::new("Dune (Paperback)");
        remove.page_count = Some(412);
        remove.language = Some("en".to_string());
        remove.pub_date = Some("1965".to_string());
        repo.create_book(&remove).unwrap();

        repo.merge_books(&keep.id, &remove.id).unwrap();

        let merged = repo.get_book(&keep.id).unwrap();
        assert_eq!(merged.page_count, Some(412));
        assert_eq!(merged.language.as_deref(), Some("en"));
        assert_eq!(merged.pub_date.as_deref(), Some("1965"));
    }

    #[test]
    fn merge_books_keeps_existing_metadata() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut keep = Book::new("Dune");
        keep.page_count = Some(300);
        repo.create_book(&keep).unwrap();

        let mut remove = Book::new("Dune (Paperback)");
        remove.page_count = Some(412);
        repo.create_book(&remove).unwrap();

        repo.merge_books(&keep.id, &remove.id).unwrap();

        let merged = repo.get_book(&keep.id).unwrap();
        assert_eq!(merged.page_count, Some(300)); // keep's value preserved
    }

    #[test]
    fn merge_books_inherits_work_id() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let work = Work::new("Dune");
        repo.create_work(&work).unwrap();

        let keep = Book::new("Dune");
        repo.create_book(&keep).unwrap();

        let mut remove = Book::new("Dune (Paperback)");
        remove.work_id = Some(work.id);
        repo.create_book(&remove).unwrap();
        // Manually set work_id since create_book already wrote it
        repo.link_book_to_work(&remove.id, &work.id).unwrap();

        repo.merge_books(&keep.id, &remove.id).unwrap();

        let merged = repo.get_book(&keep.id).unwrap();
        assert_eq!(merged.work_id, Some(work.id));
    }

    #[test]
    fn auto_group_candidates_finds_duplicates() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Dune");
        let b2 = Book::new("dune");
        let b3 = Book::new("Neuromancer");
        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();
        repo.create_book(&b3).unwrap();

        // b1 and b2 share a primary author
        let frank = Author::new("Frank Herbert");
        repo.add_book_author(&frank, &b1.id, ContributorRole::Author, 0)
            .unwrap();
        repo.add_book_author(&frank, &b2.id, ContributorRole::Author, 0)
            .unwrap();
        let gibson = Author::new("William Gibson");
        repo.add_book_author(&gibson, &b3.id, ContributorRole::Author, 0)
            .unwrap();

        let candidates = repo.auto_group_candidates().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1.len(), 2);
    }

    #[test]
    fn auto_group_excludes_already_grouped() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let work = Work::new("Dune");
        repo.create_work(&work).unwrap();

        let b1 = Book::new("Dune");
        let b2 = Book::new("Dune");
        repo.create_book(&b1).unwrap();
        repo.create_book(&b2).unwrap();

        let frank = Author::new("Frank Herbert");
        repo.add_book_author(&frank, &b1.id, ContributorRole::Author, 0)
            .unwrap();
        repo.add_book_author(&frank, &b2.id, ContributorRole::Author, 0)
            .unwrap();

        // Link b1 to work — now it shouldn't be a candidate
        repo.link_book_to_work(&b1.id, &work.id).unwrap();

        let candidates = repo.auto_group_candidates().unwrap();
        assert!(candidates.is_empty()); // only 1 ungrouped book, needs >= 2
    }

    #[test]
    fn normalize_title_strips_parentheticals() {
        assert_eq!(
            normalize_title_for_grouping("Dune (Paperback)", "Frank Herbert"),
            "dune | frank herbert"
        );
        assert_eq!(
            normalize_title_for_grouping("Dune (2nd Edition)", "Frank Herbert"),
            "dune | frank herbert"
        );
    }

    #[test]
    fn normalize_title_strips_edition_markers() {
        assert_eq!(
            normalize_title_for_grouping("Dune Kindle Edition", ""),
            "dune"
        );
        assert_eq!(
            normalize_title_for_grouping("Dune Hardcover", "Frank Herbert"),
            "dune | frank herbert"
        );
    }

    // --- Smart shelf tests ---

    #[test]
    fn create_smart_shelf_and_list() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let filter = SmartFilter::parse("status:read").unwrap();
        let shelf = repo.create_smart_shelf("Finished Books", &filter).unwrap();

        assert!(shelf.is_smart);
        assert!(shelf.smart_filter.is_some());

        let shelves = repo.list_shelves().unwrap();
        let smart = shelves.iter().find(|s| s.name == "Finished Books").unwrap();
        assert!(smart.is_smart);
        assert!(smart.smart_filter.is_some());
    }

    #[test]
    fn smart_shelf_evaluates_status_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Read Book");
        b1.status = ReadingStatus::Read;
        repo.create_book(&b1).unwrap();

        let mut b2 = Book::new("Unread Book");
        b2.status = ReadingStatus::WantToRead;
        repo.create_book(&b2).unwrap();

        let filter = SmartFilter::parse("status:read").unwrap();
        repo.create_smart_shelf("Finished", &filter).unwrap();

        let books = repo.list_books_in_shelf("Finished").unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Read Book");
    }

    #[test]
    fn smart_shelf_evaluates_rating_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Great Book");
        b1.rating = Some(9);
        repo.create_book(&b1).unwrap();

        let mut b2 = Book::new("OK Book");
        b2.rating = Some(5);
        repo.create_book(&b2).unwrap();

        let filter = SmartFilter::parse("rating:>=8").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Great Book");
    }

    #[test]
    fn smart_shelf_evaluates_pages_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Long Book");
        b1.page_count = Some(500);
        repo.create_book(&b1).unwrap();

        let mut b2 = Book::new("Short Book");
        b2.page_count = Some(100);
        repo.create_book(&b2).unwrap();

        let filter = SmartFilter::parse("pages:>300").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Long Book");
    }

    #[test]
    fn smart_shelf_evaluates_tag_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Sci-Fi Book");
        repo.create_book(&b1).unwrap();
        repo.add_tag_to_book(&b1.id, "sci-fi").unwrap();

        let b2 = Book::new("Fantasy Book");
        repo.create_book(&b2).unwrap();
        repo.add_tag_to_book(&b2.id, "fantasy").unwrap();

        let filter = SmartFilter::parse("tag:sci-fi").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Sci-Fi Book");
    }

    #[test]
    fn smart_shelf_evaluates_mood_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Dark Book");
        repo.create_book(&b1).unwrap();
        repo.add_typed_tag_to_book(&b1.id, "dark", TagType::Mood)
            .unwrap();

        let b2 = Book::new("Light Book");
        repo.create_book(&b2).unwrap();

        let filter = SmartFilter::parse("mood:dark").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Dark Book");
    }

    #[test]
    fn smart_shelf_evaluates_author_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Dune");
        repo.create_book(&b1).unwrap();
        repo.add_book_author(
            &Author::new("Frank Herbert"),
            &b1.id,
            ContributorRole::Author,
            0,
        )
        .unwrap();

        let b2 = Book::new("1984");
        repo.create_book(&b2).unwrap();
        repo.add_book_author(
            &Author::new("George Orwell"),
            &b2.id,
            ContributorRole::Author,
            0,
        )
        .unwrap();

        let filter = SmartFilter::parse("author:herbert").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Dune");
    }

    #[test]
    fn smart_shelf_evaluates_and_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Read Sci-Fi");
        b1.status = ReadingStatus::Read;
        repo.create_book(&b1).unwrap();
        repo.add_tag_to_book(&b1.id, "sci-fi").unwrap();

        let mut b2 = Book::new("Unread Sci-Fi");
        b2.status = ReadingStatus::WantToRead;
        repo.create_book(&b2).unwrap();
        repo.add_tag_to_book(&b2.id, "sci-fi").unwrap();

        let mut b3 = Book::new("Read Fantasy");
        b3.status = ReadingStatus::Read;
        repo.create_book(&b3).unwrap();
        repo.add_tag_to_book(&b3.id, "fantasy").unwrap();

        let filter = SmartFilter::parse("status:read AND tag:sci-fi").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Read Sci-Fi");
    }

    #[test]
    fn smart_shelf_evaluates_or_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Sci-Fi Book");
        repo.create_book(&b1).unwrap();
        repo.add_tag_to_book(&b1.id, "sci-fi").unwrap();

        let b2 = Book::new("Fantasy Book");
        repo.create_book(&b2).unwrap();
        repo.add_tag_to_book(&b2.id, "fantasy").unwrap();

        let b3 = Book::new("Romance Book");
        repo.create_book(&b3).unwrap();
        repo.add_tag_to_book(&b3.id, "romance").unwrap();

        let filter = SmartFilter::parse("tag:sci-fi OR tag:fantasy").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 2);
    }

    #[test]
    fn smart_shelf_evaluates_parenthesized_filter() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let mut b1 = Book::new("Unread Sci-Fi");
        b1.status = ReadingStatus::WantToRead;
        repo.create_book(&b1).unwrap();
        repo.add_tag_to_book(&b1.id, "sci-fi").unwrap();

        let mut b2 = Book::new("Unread Fantasy");
        b2.status = ReadingStatus::WantToRead;
        repo.create_book(&b2).unwrap();
        repo.add_tag_to_book(&b2.id, "fantasy").unwrap();

        let mut b3 = Book::new("Read Sci-Fi");
        b3.status = ReadingStatus::Read;
        repo.create_book(&b3).unwrap();
        repo.add_tag_to_book(&b3.id, "sci-fi").unwrap();

        let filter =
            SmartFilter::parse("status:want_to_read AND (tag:sci-fi OR tag:fantasy)").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 2);
    }

    #[test]
    fn smart_shelf_auto_updates() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let filter = SmartFilter::parse("status:read").unwrap();
        repo.create_smart_shelf("Finished", &filter).unwrap();

        // Initially empty
        let books = repo.list_books_in_shelf("Finished").unwrap();
        assert!(books.is_empty());

        // Add a read book — shelf auto-includes it
        let mut b1 = Book::new("Newly Finished");
        b1.status = ReadingStatus::Read;
        repo.create_book(&b1).unwrap();

        let books = repo.list_books_in_shelf("Finished").unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Newly Finished");
    }

    #[test]
    fn cannot_add_book_to_smart_shelf() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("Test Book");
        repo.create_book(&b1).unwrap();

        let filter = SmartFilter::parse("status:read").unwrap();
        repo.create_smart_shelf("Smart", &filter).unwrap();

        let err = repo.add_book_to_shelf(&b1.id, "Smart").unwrap_err();
        assert!(err.to_string().contains("smart shelf"));
    }

    #[test]
    fn get_shelf_by_name_test() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        repo.create_shelf("Regular").unwrap();
        let filter = SmartFilter::parse("rating:>=8").unwrap();
        repo.create_smart_shelf("Top Rated", &filter).unwrap();

        let regular = repo.get_shelf_by_name("Regular").unwrap().unwrap();
        assert!(!regular.is_smart);

        let smart = repo.get_shelf_by_name("Top Rated").unwrap().unwrap();
        assert!(smart.is_smart);
        assert!(smart.smart_filter.is_some());

        let missing = repo.get_shelf_by_name("Nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn delete_shelf_test() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        repo.create_shelf("ToDelete").unwrap();
        assert!(repo.delete_shelf("ToDelete").unwrap());
        assert!(!repo.delete_shelf("ToDelete").unwrap());

        let shelves = repo.list_shelves().unwrap();
        assert!(shelves.iter().all(|s| s.name != "ToDelete"));
    }

    #[test]
    fn smart_shelf_shelf_filter_excludes_smart_shelves() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let b1 = Book::new("On Regular Shelf");
        repo.create_book(&b1).unwrap();
        repo.add_book_to_shelf(&b1.id, "Favorites").unwrap();

        let b2 = Book::new("Not On Any Shelf");
        repo.create_book(&b2).unwrap();

        let filter = SmartFilter::parse("shelf:Favorites").unwrap();
        let books = repo.evaluate_smart_filter(&filter).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "On Regular Shelf");
    }

    #[test]
    fn smart_shelf_json_round_trip_from_db() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let filter = SmartFilter::parse("status:read AND rating:>=8").unwrap();
        repo.create_smart_shelf("Best Reads", &filter).unwrap();

        let shelf = repo.get_shelf_by_name("Best Reads").unwrap().unwrap();
        let stored_filter = SmartFilter::from_json(shelf.smart_filter.as_ref().unwrap()).unwrap();
        assert_eq!(filter, stored_filter);
    }

    // --- Soft delete tests ---

    #[test]
    fn soft_delete_hides_from_list() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("To Delete");
        repo.create_book(&book).unwrap();
        assert_eq!(repo.list_books().unwrap().len(), 1);

        assert!(repo.delete_book(&book.id).unwrap());
        assert!(repo.list_books().unwrap().is_empty());
    }

    #[test]
    fn soft_delete_hides_from_get() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Ghost Book");
        repo.create_book(&book).unwrap();
        repo.delete_book(&book.id).unwrap();

        let result = repo.get_book(&book.id);
        assert!(result.is_err()); // deleted book not found via get_book
    }

    #[test]
    fn get_book_including_deleted_returns_tombstoned_book() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Recoverable");
        repo.create_book(&book).unwrap();
        repo.delete_book(&book.id).unwrap();

        let found = repo.get_book_including_deleted(&book.id).unwrap();
        assert_eq!(found.title, "Recoverable");
    }

    #[test]
    fn soft_delete_hides_from_search() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Searchable Secret");
        repo.create_book(&book).unwrap();

        assert_eq!(repo.search_books("Searchable").unwrap().len(), 1);

        repo.delete_book(&book.id).unwrap();
        assert!(repo.search_books("Searchable").unwrap().is_empty());
    }

    #[test]
    fn soft_delete_hides_from_count_by_status() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Counter");
        repo.create_book(&book).unwrap();

        let counts = repo.count_books_by_status().unwrap();
        assert_eq!(*counts.get("want-to-read").unwrap_or(&0), 1);

        repo.delete_book(&book.id).unwrap();
        let counts = repo.count_books_by_status().unwrap();
        assert_eq!(*counts.get("want-to-read").unwrap_or(&0), 0);
    }

    #[test]
    fn soft_delete_prevents_mutation() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Immutable After Delete");
        repo.create_book(&book).unwrap();
        repo.delete_book(&book.id).unwrap();

        let updated = repo
            .update_book_status(&book.id, ReadingStatus::Reading)
            .unwrap();
        assert!(!updated); // mutation returns false for deleted book
    }

    #[test]
    fn soft_delete_is_idempotent() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Delete Twice");
        repo.create_book(&book).unwrap();

        assert!(repo.delete_book(&book.id).unwrap()); // first delete succeeds
        assert!(!repo.delete_book(&book.id).unwrap()); // second delete is no-op
    }

    #[test]
    fn purge_tombstones_removes_old_deletes() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Ancient Delete");
        repo.create_book(&book).unwrap();

        // Manually set deleted_at to 60 days ago
        let old_date = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        db.conn
            .execute(
                "UPDATE books SET deleted_at = ?1 WHERE id = ?2",
                params![old_date, book.id.to_string()],
            )
            .unwrap();

        let purged = repo.purge_tombstones(30).unwrap();
        assert_eq!(purged, 1);

        // Verify the book is truly gone (not even via including_deleted)
        assert!(repo.get_book_including_deleted(&book.id).is_err());
    }

    #[test]
    fn purge_tombstones_keeps_recent_deletes() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Recent Delete");
        repo.create_book(&book).unwrap();
        repo.delete_book(&book.id).unwrap(); // just now

        let purged = repo.purge_tombstones(30).unwrap();
        assert_eq!(purged, 0);

        // Still accessible via including_deleted
        assert!(repo.get_book_including_deleted(&book.id).is_ok());
    }

    #[test]
    fn soft_delete_hides_from_find_by_isbn() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("ISBN Book");
        repo.create_book(&book).unwrap();
        repo.add_isbn("9780000000001", &book.id).unwrap();

        assert!(repo.find_by_isbn("9780000000001").unwrap().is_some());

        repo.delete_book(&book.id).unwrap();
        assert!(repo.find_by_isbn("9780000000001").unwrap().is_none());
    }

    #[test]
    fn soft_delete_hides_from_find_by_title() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Unique Title XYZ");
        repo.create_book(&book).unwrap();

        assert!(
            repo.find_book_by_title("Unique Title XYZ")
                .unwrap()
                .is_some()
        );

        repo.delete_book(&book.id).unwrap();
        assert!(
            repo.find_book_by_title("Unique Title XYZ")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn tag_counts_exclude_deleted_books() {
        let db = test_db();
        let repo = BookRepository::new(&db);

        let book = Book::new("Tagged Book");
        repo.create_book(&book).unwrap();
        repo.add_tag_to_book(&book.id, "sci-fi").unwrap();

        let counts = repo.list_tags_with_counts().unwrap();
        let sci_fi = counts.iter().find(|(t, _)| t.name == "sci-fi").unwrap();
        assert_eq!(sci_fi.1, 1);

        repo.delete_book(&book.id).unwrap();
        let counts = repo.list_tags_with_counts().unwrap();
        let sci_fi = counts.iter().find(|(t, _)| t.name == "sci-fi").unwrap();
        assert_eq!(sci_fi.1, 0); // tag still exists but count is 0
    }
}
