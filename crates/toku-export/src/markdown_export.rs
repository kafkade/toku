use std::io::Write;

use toku_db::Database;

use crate::{BookExport, ExportError, build_library_export};

/// Export the library as a readable Markdown document grouped by reading status.
pub fn export_markdown(db: &Database, writer: impl Write) -> Result<(), ExportError> {
    let export = build_library_export(db)?;
    write_markdown(&export.books, writer)
}

fn write_markdown(books: &[BookExport], mut writer: impl Write) -> Result<(), ExportError> {
    writeln!(writer, "# My Library")?;

    let sections: &[(&str, &str)] = &[
        ("reading", "Currently Reading"),
        ("read", "Read"),
        ("want-to-read", "Want to Read"),
        ("on-hold", "On Hold"),
        ("abandoned", "Abandoned"),
    ];

    for (status_key, heading) in sections {
        let matching: Vec<&BookExport> = books.iter().filter(|b| b.status == *status_key).collect();

        if matching.is_empty() {
            continue;
        }

        writeln!(writer)?;
        writeln!(writer, "## {heading}")?;

        for book in &matching {
            let author_str = if book.authors.is_empty() {
                String::new()
            } else {
                let names: Vec<&str> = book.authors.iter().map(|a| a.name.as_str()).collect();
                format!(" by {}", names.join(", "))
            };

            let mut suffix = String::new();

            if *status_key == "reading"
                && let Some(pages) = book.page_count
            {
                suffix = format!(" ({pages} pages)");
            }

            if *status_key == "read"
                && let Some(rating) = book.rating
            {
                let stars = rating_to_stars(rating);
                let display = rating as f32 / 2.0;
                suffix = format!(" {stars} ({display:.1})");
            }

            writeln!(writer, "- **{}**{author_str}{suffix}", book.title)?;
        }
    }

    Ok(())
}

/// Convert a 0–10 integer rating to a star display (e.g. 8 → ★★★★).
fn rating_to_stars(rating: i32) -> String {
    let full = rating / 2;
    let half = rating % 2;
    let mut s = String::new();
    for _ in 0..full {
        s.push('★');
    }
    if half > 0 {
        s.push('½');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use toku_core::{Author, Book, BookFormat, ContributorRole, ReadingStatus};
    use toku_db::BookRepository;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let repo = BookRepository::new(&db);

        let mut book1 = Book::new("Dune");
        book1.status = ReadingStatus::Reading;
        book1.page_count = Some(544);
        book1.format = BookFormat::Physical;
        repo.create_book(&book1).unwrap();
        let a1 = Author::new("Frank Herbert");
        repo.add_book_author(&a1, &book1.id, ContributorRole::Author, 0)
            .unwrap();

        let mut book2 = Book::new("Neuromancer");
        book2.status = ReadingStatus::Read;
        book2.rating = Some(8);
        book2.format = BookFormat::Physical;
        repo.create_book(&book2).unwrap();
        let a2 = Author::new("William Gibson");
        repo.add_book_author(&a2, &book2.id, ContributorRole::Author, 0)
            .unwrap();

        let mut book3 = Book::new("Snow Crash");
        book3.status = ReadingStatus::Read;
        book3.rating = Some(10);
        book3.format = BookFormat::Ebook;
        repo.create_book(&book3).unwrap();
        let a3 = Author::new("Neal Stephenson");
        repo.add_book_author(&a3, &book3.id, ContributorRole::Author, 0)
            .unwrap();

        let mut book4 = Book::new("The Left Hand of Darkness");
        book4.status = ReadingStatus::WantToRead;
        book4.format = BookFormat::Physical;
        repo.create_book(&book4).unwrap();
        let a4 = Author::new("Ursula K. Le Guin");
        repo.add_book_author(&a4, &book4.id, ContributorRole::Author, 0)
            .unwrap();

        db
    }

    #[test]
    fn markdown_has_headings() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_markdown(&db, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("# My Library"));
        assert!(output.contains("## Currently Reading"));
        assert!(output.contains("## Read"));
        assert!(output.contains("## Want to Read"));
    }

    #[test]
    fn markdown_contains_books() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_markdown(&db, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("**Dune**"));
        assert!(output.contains("Frank Herbert"));
        assert!(output.contains("**Neuromancer**"));
        assert!(output.contains("**The Left Hand of Darkness**"));
    }

    #[test]
    fn markdown_shows_ratings_for_read() {
        let db = setup_db();
        let mut buf = Vec::new();
        export_markdown(&db, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        // Neuromancer rating=8 → 4.0 stars
        assert!(output.contains("★★★★ (4.0)"));
        // Snow Crash rating=10 → 5.0 stars
        assert!(output.contains("★★★★★ (5.0)"));
    }

    #[test]
    fn markdown_omits_empty_sections() {
        let db = Database::open_in_memory().unwrap();
        let repo = BookRepository::new(&db);

        let mut book = Book::new("Test Book");
        book.status = ReadingStatus::Read;
        book.rating = Some(6);
        book.format = BookFormat::Physical;
        repo.create_book(&book).unwrap();

        let mut buf = Vec::new();
        export_markdown(&db, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("## Read"));
        assert!(!output.contains("## Currently Reading"));
        assert!(!output.contains("## Want to Read"));
        assert!(!output.contains("## On Hold"));
        assert!(!output.contains("## Abandoned"));
    }
}
