use std::io::Write;

use toku_db::Database;

use crate::{ExportError, build_library_export};

/// Export books as a flat CSV.
///
/// Columns: title, authors, status, format, rating, pages, pub_date, shelves, tags, isbn
pub fn export_csv(db: &Database, writer: impl Write) -> Result<(), ExportError> {
    let export = build_library_export(db)?;

    let mut wtr = csv::Writer::from_writer(writer);

    wtr.write_record([
        "title", "authors", "status", "format", "rating", "pages", "pub_date", "shelves", "tags",
        "isbn",
    ])?;

    for book in &export.books {
        let authors = book
            .authors
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("; ");

        let shelves = book.shelves.join("; ");
        let tags = book.tags.join("; ");

        let isbn = book
            .isbn_13
            .as_deref()
            .or(book.isbn_10.as_deref())
            .unwrap_or("");

        wtr.write_record([
            &book.title,
            &authors,
            &book.status,
            &book.format,
            &book.rating.map_or(String::new(), |r| r.to_string()),
            &book.page_count.map_or(String::new(), |p| p.to_string()),
            book.pub_date.as_deref().unwrap_or(""),
            &shelves,
            &tags,
            isbn,
        ])?;
    }

    wtr.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use toku_core::{Author, Book, BookFormat, ContributorRole, ReadingStatus};
    use toku_db::BookRepository;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let repo = BookRepository::new(&db);

        let mut book = Book::new("Dune");
        book.page_count = Some(544);
        book.pub_date = Some("1965".to_string());
        book.status = ReadingStatus::Read;
        book.rating = Some(9);
        book.format = BookFormat::Physical;
        repo.create_book(&book).unwrap();

        let author = Author::new("Frank Herbert");
        repo.add_book_author(&author, &book.id, ContributorRole::Author, 0)
            .unwrap();

        repo.add_isbn("9780441013593", &book.id).unwrap();

        let mut book2 = Book::new("Neuromancer");
        book2.status = ReadingStatus::WantToRead;
        book2.format = BookFormat::Ebook;
        repo.create_book(&book2).unwrap();

        let author2 = Author::new("William Gibson");
        repo.add_book_author(&author2, &book2.id, ContributorRole::Author, 0)
            .unwrap();

        db
    }

    #[test]
    fn csv_has_correct_columns() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_csv(&db, &mut buf).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let mut rdr = csv::Reader::from_reader(output.as_bytes());
        let headers = rdr.headers().unwrap();
        assert_eq!(
            headers,
            &csv::StringRecord::from(vec![
                "title", "authors", "status", "format", "rating", "pages", "pub_date", "shelves",
                "tags", "isbn"
            ])
        );
    }

    #[test]
    fn csv_contains_books() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_csv(&db, &mut buf).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let mut rdr = csv::Reader::from_reader(output.as_bytes());
        let records: Vec<csv::StringRecord> = rdr.records().filter_map(|r| r.ok()).collect();
        assert_eq!(records.len(), 2);

        // Books are ordered by title; Dune comes first
        assert_eq!(&records[0][0], "Dune");
        assert_eq!(&records[0][1], "Frank Herbert");
        assert_eq!(&records[0][2], "read");
        assert_eq!(&records[0][9], "9780441013593");

        assert_eq!(&records[1][0], "Neuromancer");
        assert_eq!(&records[1][1], "William Gibson");
    }

    #[test]
    fn csv_produces_valid_csv() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_csv(&db, &mut buf).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let mut rdr = csv::Reader::from_reader(output.as_bytes());
        // All records should parse without error
        for result in rdr.records() {
            result.unwrap();
        }
    }
}
