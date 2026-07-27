mod backup;
mod csv_export;
mod error;
mod json_export;
mod markdown_export;

pub use backup::{export_backup, import_backup, read_backup_manifest};
pub use csv_export::export_csv;
pub use error::ExportError;
pub use json_export::export_json;
pub use markdown_export::export_markdown;

use serde::{Deserialize, Serialize};

/// Full library export with metadata envelope.
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryExport {
    pub version: String,
    pub exported_at: String,
    pub book_count: usize,
    pub books: Vec<BookExport>,
}

/// A single book with all related data flattened for export.
#[derive(Debug, Serialize, Deserialize)]
pub struct BookExport {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<AuthorExport>,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub page_count: Option<i32>,
    pub pub_date: Option<String>,
    pub language: Option<String>,
    pub format: String,
    pub status: String,
    pub rating: Option<i32>,
    pub shelves: Vec<String>,
    pub tags: Vec<String>,
    pub cover_hash: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// An author/contributor entry for export.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorExport {
    pub name: String,
    pub role: String,
}

use toku_db::{BookRepository, Database};

/// Build a full `LibraryExport` from the database.
pub fn build_library_export(db: &Database) -> Result<LibraryExport, ExportError> {
    let repo = BookRepository::new(db);
    let books = repo.list_books()?;

    let mut book_exports = Vec::with_capacity(books.len());
    for book in &books {
        let authors_raw = repo.get_book_authors(&book.id)?;
        let authors: Vec<AuthorExport> = authors_raw
            .into_iter()
            .map(|(a, ba)| AuthorExport {
                name: a.name,
                role: ba.role.as_str().to_string(),
            })
            .collect();

        let shelves: Vec<String> = repo
            .get_book_shelves(&book.id)?
            .into_iter()
            .map(|s| s.name)
            .collect();

        let tags: Vec<String> = repo
            .get_book_tags(&book.id)?
            .into_iter()
            .map(|t| t.name)
            .collect();

        let isbns = repo.get_book_isbns(&book.id)?;
        let isbn_13 = isbns.iter().find(|i| i.len() == 13).cloned();
        let isbn_10 = isbns.iter().find(|i| i.len() == 10).cloned();

        book_exports.push(BookExport {
            id: book.id.to_string(),
            title: book.title.clone(),
            subtitle: book.subtitle.clone(),
            description: book.description.clone(),
            authors,
            isbn_13,
            isbn_10,
            page_count: book.page_count,
            pub_date: book.pub_date.clone(),
            language: book.language.clone(),
            format: book.format.as_str().to_string(),
            status: book.status.as_str().to_string(),
            rating: book.rating,
            shelves,
            tags,
            cover_hash: book.cover_hash.clone(),
            created_at: book.created_at.to_rfc3339(),
            updated_at: book.updated_at.to_rfc3339(),
        });
    }

    Ok(LibraryExport {
        version: "1".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        book_count: book_exports.len(),
        books: book_exports,
    })
}
