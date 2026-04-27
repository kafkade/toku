use rusqlite::params;
use toku_core::{
    Author, Book, BookAuthor, ContributorRole, ReadingSession, ReadingStatus, Shelf, Tag,
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
        match self.db.conn.query_row(
            "SELECT id, name, created_at FROM tags WHERE name = ?1",
            params![name],
            |row| {
                let id_str: String = row.get(0)?;
                let tag_name: String = row.get(1)?;
                let created_str: String = row.get(2)?;
                Ok((id_str, tag_name, created_str))
            },
        ) {
            Ok((id_str, tag_name, created_str)) => {
                let tag = Tag {
                    id: Uuid::parse_str(&id_str).map_err(|e| DbError::Io(e.to_string()))?,
                    name: tag_name,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .map_err(|e| DbError::Io(e.to_string()))?
                        .with_timezone(&chrono::Utc),
                };
                Ok(tag)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let tag = Tag::new(name);
                self.db.conn.execute(
                    "INSERT INTO tags (id, name, created_at) VALUES (?1, ?2, ?3)",
                    params![tag.id.to_string(), tag.name, tag.created_at.to_rfc3339(),],
                )?;
                Ok(tag)
            }
            Err(e) => Err(DbError::Sqlite(e)),
        }
    }

    /// Add a tag to a book, creating the tag if it doesn't exist.
    pub fn add_tag_to_book(&self, book_id: &Uuid, tag_name: &str) -> Result<(), DbError> {
        self.get_book(book_id)?;
        let tag = self.create_tag(tag_name)?;

        self.db.conn.execute(
            "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
            params![book_id.to_string(), tag.id.to_string()],
        )?;

        Ok(())
    }

    /// Remove a tag from a book.
    pub fn remove_tag_from_book(&self, book_id: &Uuid, tag_name: &str) -> Result<(), DbError> {
        let rows = self.db.conn.execute(
            "DELETE FROM book_tags WHERE book_id = ?1
             AND tag_id IN (SELECT id FROM tags WHERE name = ?2)",
            params![book_id.to_string(), tag_name],
        )?;

        if rows == 0 {
            return Err(DbError::NotFound(format!(
                "tag '{tag_name}' not on this book"
            )));
        }

        Ok(())
    }

    /// Get all tags for a book.
    pub fn get_book_tags(&self, book_id: &Uuid) -> Result<Vec<Tag>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.created_at
             FROM tags t
             JOIN book_tags bt ON bt.tag_id = t.id
             WHERE bt.book_id = ?1
             ORDER BY t.name COLLATE NOCASE",
        )?;

        let tags = stmt
            .query_map(params![book_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let created_str: String = row.get(2)?;
                Ok((id_str, name, created_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, created_str)| {
                Some(Tag {
                    id: Uuid::parse_str(&id_str).ok()?,
                    name,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect();

        Ok(tags)
    }

    /// List all books with a given tag.
    pub fn list_books_by_tag(&self, tag_name: &str) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM books b
             JOIN book_tags bt ON bt.book_id = b.id
             JOIN tags t ON t.id = bt.tag_id
             WHERE t.name = ?1
             ORDER BY b.title COLLATE NOCASE",
        )?;

        let books = stmt
            .query_map(params![tag_name], |row| Ok(row_to_book(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();

        Ok(books)
    }

    /// List all tags with book counts.
    pub fn list_tags_with_counts(&self) -> Result<Vec<(Tag, i64)>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT t.id, t.name, t.created_at, COUNT(bt.book_id) as book_count
             FROM tags t
             LEFT JOIN book_tags bt ON bt.tag_id = t.id
             GROUP BY t.id
             ORDER BY t.name COLLATE NOCASE",
        )?;

        let tags = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let name: String = row.get(1)?;
                let created_str: String = row.get(2)?;
                let count: i64 = row.get(3)?;
                Ok((id_str, name, created_str, count))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, name, created_str, count)| {
                Some((
                    Tag {
                        id: Uuid::parse_str(&id_str).ok()?,
                        name,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use toku_core::ReadingStatus;

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
}
