use rusqlite::params;
use toku_core::{Author, Book, BookAuthor, ContributorRole};
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
        self.db.conn.execute(
            "INSERT INTO books (id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status, rating,
             created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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

    /// Full-text search across title, subtitle, description.
    pub fn search_books(&self, query: &str) -> Result<Vec<Book>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT b.id, b.title, b.subtitle, b.description, b.page_count, b.pub_date,
             b.language, b.format, b.duration_minutes, b.cover_hash, b.work_id, b.status,
             b.rating, b.created_at, b.updated_at
             FROM books_fts f
             JOIN books b ON b.rowid = f.rowid
             WHERE books_fts MATCH ?1
             ORDER BY rank",
        )?;

        let books = stmt
            .query_map(params![query], |row| Ok(row_to_book(row)))?
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
}
