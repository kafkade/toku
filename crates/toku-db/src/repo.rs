use rusqlite::params;
use toku_core::{
    Author, AuthorCount, Book, BookAuthor, ContributorRole, PaceRating, ReadingProgress,
    ReadingSession, ReadingStatus, Shelf, Tag, TagCount, TagType,
};
use uuid::Uuid;

use crate::{Database, DbError};

/// Book persistence operations.
pub struct BookRepository<'a> {
    db: &'a Database,
}

impl<'a> BookRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert a new book. Returns the book's ID.
    pub fn create_book(&self, book: &Book) -> Result<(), DbError> {
        let search_text = build_search_text(
            &book.title,
            book.subtitle.as_deref(),
            book.description.as_deref(),
            &[],
        );
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
        Ok(())
    }

    /// Retrieve a book by its UUID.
    pub fn get_book(&self, id: &Uuid) -> Result<Book, DbError> {
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

    /// List all books, ordered by title.
    pub fn list_books(&self) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status,
             rating, created_at, updated_at
             FROM books ORDER BY title COLLATE NOCASE",
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
             WHERE books_fts MATCH ?1",
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

    /// Delete a book by ID.
    pub fn delete_book(&self, id: &Uuid) -> Result<bool, DbError> {
        let rows = self
            .db
            .conn
            .execute("DELETE FROM books WHERE id = ?1", params![id.to_string()])?;
        Ok(rows > 0)
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
             WHERE i.isbn = ?1",
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
             FROM books WHERE title = ?1 COLLATE NOCASE",
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
        let rows = self.db.conn.execute(
            "UPDATE books SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                status.as_str(),
                chrono::Utc::now().to_rfc3339(),
                id.to_string(),
            ],
        )?;
        Ok(rows > 0)
    }

    /// Update a book's rating.
    pub fn update_book_rating(&self, id: &Uuid, rating: i32) -> Result<bool, DbError> {
        let rows = self.db.conn.execute(
            "UPDATE books SET rating = ?1, updated_at = ?2 WHERE id = ?3",
            params![rating, chrono::Utc::now().to_rfc3339(), id.to_string(),],
        )?;
        Ok(rows > 0)
    }

    /// Insert a new reading session.
    pub fn create_reading_session(&self, session: &ReadingSession) -> Result<(), DbError> {
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
        Ok(())
    }

    // --- Reading progress operations ---

    /// Insert a reading progress entry.
    pub fn log_progress(&self, progress: &ReadingProgress) -> Result<(), DbError> {
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
        Ok(())
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
            "INSERT INTO shelves (id, name, created_at) VALUES (?1, ?2, ?3)",
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
            .prepare("SELECT id, name, created_at FROM shelves ORDER BY name COLLATE NOCASE")?;

        let shelves = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let created_str: String = row.get(2)?;
                Ok((id_str, name, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, created_str)| {
                Some(Shelf {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(shelves)
    }

    /// Add a book to a shelf, creating the shelf if it doesn't exist.
    pub fn add_book_to_shelf(&self, book_id: &Uuid, shelf_name: &str) -> Result<(), DbError> {
        self.get_book(book_id)?;

        let shelf_id: String = match self.db.conn.query_row(
            "SELECT id FROM shelves WHERE name = ?1",
            params![shelf_name],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let shelf = self.create_shelf(shelf_name)?;
                shelf.id.to_string()
            }
            Err(e) => return Err(DbError::Sqlite(e)),
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
            "SELECT s.id, s.name, s.created_at
             FROM shelves s
             JOIN book_shelves bs ON bs.shelf_id = s.id
             WHERE bs.book_id = ?1
             ORDER BY s.name COLLATE NOCASE",
        )?;

        let shelves = stmt
            .query_map(params![book_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let created_str: String = row.get(2)?;
                Ok((id_str, name, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, created_str)| {
                Some(Shelf {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(shelves)
    }

    /// List all books in a shelf.
    pub fn list_books_in_shelf(&self, shelf_name: &str) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM books b
             JOIN book_shelves bs ON bs.book_id = b.id
             JOIN shelves s ON s.id = bs.shelf_id
             WHERE s.name = ?1
             ORDER BY b.title COLLATE NOCASE",
        )?;

        let books = stmt
            .query_map(params![shelf_name], |row| Ok(row_to_book(row)))?
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

        self.db.conn.execute(
            "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
            params![book_id.to_string(), tag.id.to_string()],
        )?;

        Ok(())
    }

    /// Set the pace rating for a book. Removes any existing pace tag first.
    pub fn set_book_pace(&self, book_id: &Uuid, pace: PaceRating) -> Result<(), DbError> {
        self.get_book(book_id)?;

        // Remove existing pace tags
        self.db.conn.execute(
            "DELETE FROM book_tags WHERE book_id = ?1
             AND tag_id IN (SELECT id FROM tags WHERE tag_type = 'pace')",
            params![book_id.to_string()],
        )?;

        // Add new pace tag
        self.add_typed_tag_to_book(book_id, pace.as_str(), TagType::Pace)
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

        Ok(())
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
             WHERE t.name = ?1 AND t.tag_type = ?2
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

    /// List all tags with book counts.
    pub fn list_tags_with_counts(&self) -> Result<Vec<(Tag, i64)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.tag_type, t.created_at, COUNT(bt.book_id) as book_count
             FROM tags t
             LEFT JOIN book_tags bt ON bt.tag_id = t.id
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

    /// List tags of a specific type with book counts.
    pub fn list_tags_with_counts_by_type(
        &self,
        tag_type: TagType,
    ) -> Result<Vec<(Tag, i64)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.tag_type, t.created_at, COUNT(bt.book_id) as book_count
             FROM tags t
             LEFT JOIN book_tags bt ON bt.tag_id = t.id
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
        let mut stmt = self
            .db
            .conn
            .prepare("SELECT status, COUNT(*) FROM books GROUP BY status")?;

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
             FROM books WHERE status = 'reading'
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
             WHERE LOWER(a.name) = LOWER(?1)
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
}
