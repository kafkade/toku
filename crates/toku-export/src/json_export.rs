use std::io::Write;

use toku_db::Database;

use crate::{ExportError, build_library_export};

/// Export the full library as pretty-printed JSON.
pub fn export_json(db: &Database, writer: impl Write) -> Result<(), ExportError> {
    let export = build_library_export(db)?;
    serde_json::to_writer_pretty(writer, &export)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibraryExport;
    use toku_core::{Author, Book, BookFormat, ContributorRole, ReadingStatus};
    use toku_db::BookRepository;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let repo = BookRepository::new(&db);

        let mut book = Book::new("Dune");
        book.page_count = Some(544);
        book.pub_date = Some("1965".to_string());
        book.status = ReadingStatus::Read;
        book.rating = Some(8);
        book.format = BookFormat::Physical;
        repo.create_book(&book).unwrap();

        let author = Author::new("Frank Herbert");
        repo.add_book_author(&author, &book.id, ContributorRole::Author, 0)
            .unwrap();

        repo.add_isbn("9780441013593", &book.id).unwrap();

        repo.add_book_to_shelf(&book.id, "Favorites").unwrap();
        repo.add_tag_to_book(&book.id, "sci-fi").unwrap();

        db
    }

    #[test]
    fn json_round_trips() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_json(&db, &mut buf).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let parsed: LibraryExport = serde_json::from_str(&output).unwrap();

        assert_eq!(parsed.version, "1");
        assert_eq!(parsed.book_count, 1);
        assert_eq!(parsed.books.len(), 1);

        let book = &parsed.books[0];
        assert_eq!(book.title, "Dune");
        assert_eq!(book.status, "read");
        assert_eq!(book.rating, Some(8));
        assert_eq!(book.page_count, Some(544));
        assert_eq!(book.isbn_13.as_deref(), Some("9780441013593"));
        assert_eq!(book.authors.len(), 1);
        assert_eq!(book.authors[0].name, "Frank Herbert");
        assert_eq!(book.authors[0].role, "author");
        assert_eq!(book.shelves, vec!["Favorites"]);
        assert_eq!(book.tags, vec!["sci-fi"]);
    }

    #[test]
    fn json_empty_library() {
        let db = Database::open_in_memory().unwrap();
        let mut buf = Vec::new();
        export_json(&db, &mut buf).unwrap();

        let parsed: LibraryExport = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed.book_count, 0);
        assert!(parsed.books.is_empty());
    }
}
