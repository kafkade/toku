use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use csv::ReaderBuilder;
use rusqlite::params;
use toku_core::{Author, Book, BookFormat, ContributorRole, Isbn, ReadingStatus};
use toku_db::{BookRepository, Database};
use uuid::Uuid;

use crate::ImportError;

/// Options for a Goodreads import.
pub struct GoodreadsImportOptions {
    pub dry_run: bool,
}

/// The outcome of processing a single row.
#[derive(Debug, Clone)]
pub enum RowOutcome {
    Imported,
    Skipped,
    Error(String),
}

/// Progress event emitted for each row during import.
#[derive(Debug, Clone)]
pub struct ImportEvent {
    pub row: usize,
    pub total: usize,
    pub title: String,
    pub author: String,
    pub status: String,
    pub outcome: RowOutcome,
}

/// Trait for observing import progress. Implement this to receive per-row
/// events during an import (e.g. to drive a progress bar).
pub trait ImportObserver {
    fn on_event(&mut self, event: &ImportEvent) -> Result<(), ImportError>;
}

/// A short summary of a skipped or imported row, kept for the final report.
#[derive(Debug, Clone)]
pub struct RowSummary {
    pub title: String,
    pub author: String,
    pub status: String,
}

const MAX_REPORT_SAMPLES: usize = 20;

/// Summary report of an import operation.
#[derive(Debug, Default)]
pub struct ImportReport {
    pub total_rows: usize,
    pub imported: usize,
    pub skipped: usize,
    pub errors: usize,
    pub error_details: Vec<String>,
    pub import_id: Option<String>,
    /// Bounded sample of imported books (capped at 20).
    pub imported_samples: Vec<RowSummary>,
    /// Bounded sample of skipped (duplicate) books (capped at 20).
    pub skipped_samples: Vec<RowSummary>,
    /// Counts of imported books by reading status.
    pub status_counts: HashMap<String, usize>,
}

impl std::fmt::Display for ImportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Import summary:")?;
        writeln!(f, "  Total rows:  {}", self.total_rows)?;
        writeln!(f, "  Imported:    {}", self.imported)?;
        writeln!(f, "  Skipped:     {} (already in library)", self.skipped)?;
        writeln!(f, "  Errors:      {}", self.errors)?;
        if !self.error_details.is_empty() {
            writeln!(f, "  Error details:")?;
            for (i, e) in self.error_details.iter().enumerate().take(10) {
                writeln!(f, "    {}: {e}", i + 1)?;
            }
            if self.error_details.len() > 10 {
                writeln!(f, "    ... and {} more", self.error_details.len() - 10)?;
            }
        }
        Ok(())
    }
}

/// Count the number of data rows in a CSV file (excludes the header).
fn count_csv_rows(csv_path: &Path) -> Result<usize, ImportError> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(csv_path)?;
    Ok(rdr.records().count())
}

/// Import books from a Goodreads CSV export.
///
/// If an `observer` is provided, it will be called with progress events for
/// each row processed, enabling progress bars and live feedback.
pub fn import_goodreads(
    db: &Database,
    csv_path: &Path,
    opts: &GoodreadsImportOptions,
    observer: Option<&mut dyn ImportObserver>,
) -> Result<ImportReport, ImportError> {
    let total_rows = count_csv_rows(csv_path)?;
    import_goodreads_inner(db, csv_path, opts, observer, total_rows)
}

fn import_goodreads_inner(
    db: &Database,
    csv_path: &Path,
    opts: &GoodreadsImportOptions,
    mut observer: Option<&mut dyn ImportObserver>,
    total_rows: usize,
) -> Result<ImportReport, ImportError> {
    let repo = BookRepository::new(db);
    let mut report = ImportReport::default();

    let import_id = Uuid::now_v7().to_string();

    // Wrap non-dry-run imports in a transaction for atomicity
    if !opts.dry_run {
        db.conn.execute_batch("BEGIN IMMEDIATE")?;

        // Create import log entry (rows will reference this via FK)
        db.conn.execute(
            "INSERT INTO import_logs (id, source, file_path, started_at, total_rows)
             VALUES (?1, 'goodreads', ?2, ?3, 0)",
            params![
                import_id,
                csv_path.to_string_lossy().to_string(),
                Utc::now().to_rfc3339()
            ],
        )?;
    }

    let result = import_rows(
        db,
        &repo,
        csv_path,
        opts,
        &mut report,
        &import_id,
        &mut observer,
        total_rows,
    );

    if !opts.dry_run {
        match &result {
            Ok(()) => {
                // Finalize import log and commit
                db.conn.execute(
                    "UPDATE import_logs SET finished_at = ?1, total_rows = ?2,
                     imported = ?3, skipped = ?4, errors = ?5 WHERE id = ?6",
                    params![
                        Utc::now().to_rfc3339(),
                        report.total_rows as i64,
                        report.imported as i64,
                        report.skipped as i64,
                        report.errors as i64,
                        import_id,
                    ],
                )?;
                report.import_id = Some(import_id);
                db.conn.execute_batch("COMMIT")?;
            }
            Err(_) => {
                db.conn.execute_batch("ROLLBACK").ok();
            }
        }
    }

    result?;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn import_rows(
    db: &Database,
    repo: &BookRepository,
    csv_path: &Path,
    opts: &GoodreadsImportOptions,
    report: &mut ImportReport,
    import_id: &str,
    observer: &mut Option<&mut dyn ImportObserver>,
    total_rows: usize,
) -> Result<(), ImportError> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(csv_path)?;

    let headers = rdr.headers()?.clone();
    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| h == name) };

    // Map expected Goodreads columns to positions
    let col_book_id = col("Book Id");
    let col_title = col("Title");
    let col_author = col("Author");
    let col_additional_authors = col("Additional Authors");
    let col_isbn = col("ISBN");
    let col_isbn13 = col("ISBN13");
    let col_rating = col("My Rating");
    let _col_publisher = col("Publisher");
    let col_binding = col("Binding");
    let col_pages = col("Number of Pages");
    let col_year_pub = col("Year Published");
    let col_orig_year = col("Original Publication Year");
    let _col_date_read = col("Date Read");
    let _col_date_added = col("Date Added");
    let _col_shelves = col("Bookshelves");
    let col_exclusive = col("Exclusive Shelf");
    let _col_review = col("My Review");
    let _col_read_count = col("Read Count");

    if col_title.is_none() {
        return Err(ImportError::Other(
            "CSV does not contain a 'Title' column — is this a Goodreads export?".to_string(),
        ));
    }

    // Build a set of existing goodreads_ids for fast dedup
    let mut existing_gr_ids: HashMap<String, Uuid> = HashMap::new();
    {
        let mut stmt = db
            .conn
            .prepare("SELECT goodreads_id, id FROM books WHERE goodreads_id IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            let gr_id: String = row.get(0)?;
            let id_str: String = row.get(1)?;
            Ok((gr_id, Uuid::parse_str(&id_str).unwrap_or_default()))
        })?;
        for row in rows.flatten() {
            existing_gr_ids.insert(row.0, row.1);
        }
    }

    for result in rdr.records() {
        report.total_rows += 1;

        let record = match result {
            Ok(r) => r,
            Err(e) => {
                let err_msg = format!("Row {}: CSV parse error: {e}", report.total_rows);
                report.errors += 1;
                if report.error_details.len() < MAX_REPORT_SAMPLES {
                    report.error_details.push(err_msg);
                }
                emit_event(
                    observer,
                    report.total_rows,
                    total_rows,
                    "",
                    "",
                    "",
                    RowOutcome::Error(format!("CSV parse error: {e}")),
                )?;
                continue;
            }
        };

        let get = |col: Option<usize>| -> Option<String> {
            col.and_then(|i| record.get(i))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        let title = match get(col_title) {
            Some(t) => t,
            None => {
                report.errors += 1;
                if report.error_details.len() < MAX_REPORT_SAMPLES {
                    report
                        .error_details
                        .push(format!("Row {}: missing title", report.total_rows));
                }
                emit_event(
                    observer,
                    report.total_rows,
                    total_rows,
                    "",
                    "",
                    "",
                    RowOutcome::Error("missing title".to_string()),
                )?;
                continue;
            }
        };

        let gr_id = get(col_book_id);
        let author_name = get(col_author).unwrap_or_default();
        let status_str = get(col_exclusive)
            .map(|s| map_shelf_to_status(&s).as_str().to_string())
            .unwrap_or_else(|| "want-to-read".to_string());

        // Dedup: skip if goodreads_id already exists
        if let Some(ref id) = gr_id
            && existing_gr_ids.contains_key(id)
        {
            report.skipped += 1;
            if report.skipped_samples.len() < MAX_REPORT_SAMPLES {
                report.skipped_samples.push(RowSummary {
                    title: title.clone(),
                    author: author_name.clone(),
                    status: status_str.clone(),
                });
            }
            emit_event(
                observer,
                report.total_rows,
                total_rows,
                &title,
                &author_name,
                &status_str,
                RowOutcome::Skipped,
            )?;
            continue;
        }

        if opts.dry_run {
            report.imported += 1;
            *report.status_counts.entry(status_str.clone()).or_insert(0) += 1;
            if report.imported_samples.len() < MAX_REPORT_SAMPLES {
                report.imported_samples.push(RowSummary {
                    title: title.clone(),
                    author: author_name.clone(),
                    status: status_str.clone(),
                });
            }
            emit_event(
                observer,
                report.total_rows,
                total_rows,
                &title,
                &author_name,
                &status_str,
                RowOutcome::Imported,
            )?;
            continue;
        }

        // Build the book
        let mut book = Book::new(&title);

        // ISBN
        let isbn13_raw = get(col_isbn13).map(|s| clean_isbn(&s));
        let isbn_raw = get(col_isbn).map(|s| clean_isbn(&s));

        // Status from Exclusive Shelf
        if let Some(shelf) = get(col_exclusive) {
            book.status = map_shelf_to_status(&shelf);
        }

        // Rating (Goodreads 0-5 → Toku 0-10)
        if let Some(rating_str) = get(col_rating)
            && let Ok(r) = rating_str.parse::<i32>()
            && r > 0
        {
            book.rating = Some(r * 2);
        }

        // Page count
        if let Some(pages_str) = get(col_pages) {
            book.page_count = pages_str.parse().ok();
        }

        // Publication date
        if let Some(year) = get(col_orig_year).or_else(|| get(col_year_pub)) {
            book.pub_date = Some(year);
        }

        // Format from Binding
        if let Some(binding) = get(col_binding) {
            book.format = map_binding_to_format(&binding);
        }

        // Insert book with goodreads_id
        db.conn.execute(
            "INSERT INTO books (id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status, rating,
             created_at, updated_at, goodreads_id)
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
                gr_id,
            ],
        )?;

        // Store ISBNs
        if let Some(ref isbn13) = isbn13_raw
            && isbn13.len() == 13
            && let Ok(validated) = Isbn::parse(isbn13)
        {
            repo.add_isbn(validated.as_str(), &book.id)?;
        }
        if let Some(ref isbn) = isbn_raw
            && (isbn.len() == 10 || isbn.len() == 13)
            && let Ok(validated) = Isbn::parse(isbn)
        {
            let isbn_str = validated.as_str().to_string();
            // Avoid duplicate if same as isbn13
            if Some(&isbn_str) != isbn13_raw.as_ref() {
                let _ = repo.add_isbn(&isbn_str, &book.id);
            }
        }

        // Author
        if let Some(ref author_name) = get(col_author) {
            let a = Author::new(author_name.as_str());
            repo.add_book_author(&a, &book.id, ContributorRole::Author, 0)?;
        }

        // Additional authors
        if let Some(additional) = get(col_additional_authors) {
            for (i, name) in additional.split(',').enumerate() {
                let name = name.trim();
                if !name.is_empty() {
                    let a = Author::new(name);
                    repo.add_book_author(&a, &book.id, ContributorRole::Author, (i + 1) as i32)?;
                }
            }
        }

        // Track import → book relationship
        db.conn.execute(
            "INSERT INTO import_books (import_id, book_id) VALUES (?1, ?2)",
            params![import_id, book.id.to_string()],
        )?;

        // Store provenance
        set_provenance(&db.conn, &book.id, "title", "goodreads_import")?;
        if book.rating.is_some() {
            set_provenance(&db.conn, &book.id, "rating", "goodreads_import")?;
        }
        if book.page_count.is_some() {
            set_provenance(&db.conn, &book.id, "page_count", "goodreads_import")?;
        }
        if book.pub_date.is_some() {
            set_provenance(&db.conn, &book.id, "pub_date", "goodreads_import")?;
        }

        if let Some(ref id) = gr_id {
            existing_gr_ids.insert(id.clone(), book.id);
        }

        let book_status = book.status.as_str().to_string();
        report.imported += 1;
        *report.status_counts.entry(book_status.clone()).or_insert(0) += 1;
        if report.imported_samples.len() < MAX_REPORT_SAMPLES {
            report.imported_samples.push(RowSummary {
                title: title.clone(),
                author: author_name.clone(),
                status: book_status.clone(),
            });
        }

        emit_event(
            observer,
            report.total_rows,
            total_rows,
            &title,
            &author_name,
            &book_status,
            RowOutcome::Imported,
        )?;
    }

    Ok(())
}

fn emit_event(
    observer: &mut Option<&mut dyn ImportObserver>,
    row: usize,
    total: usize,
    title: &str,
    author: &str,
    status: &str,
    outcome: RowOutcome,
) -> Result<(), ImportError> {
    if let Some(obs) = observer {
        obs.on_event(&ImportEvent {
            row,
            total,
            title: title.to_string(),
            author: author.to_string(),
            status: status.to_string(),
            outcome,
        })?;
    }
    Ok(())
}

/// Undo an import by removing all books added by it.
pub fn undo_import(db: &Database, import_id: &str) -> Result<usize, ImportError> {
    let count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM import_books WHERE import_id = ?1",
        params![import_id],
        |row| row.get(0),
    )?;

    db.conn.execute(
        "DELETE FROM books WHERE id IN (SELECT book_id FROM import_books WHERE import_id = ?1)",
        params![import_id],
    )?;

    db.conn
        .execute("DELETE FROM import_logs WHERE id = ?1", params![import_id])?;

    Ok(count as usize)
}

fn map_shelf_to_status(shelf: &str) -> ReadingStatus {
    match shelf.to_lowercase().as_str() {
        "read" => ReadingStatus::Read,
        "currently-reading" => ReadingStatus::Reading,
        "to-read" => ReadingStatus::WantToRead,
        _ => ReadingStatus::WantToRead,
    }
}

fn map_binding_to_format(binding: &str) -> BookFormat {
    match binding.to_lowercase().as_str() {
        "kindle edition" | "ebook" => BookFormat::Ebook,
        "audiobook" | "audio cd" | "audible audio" => BookFormat::Audiobook,
        _ => BookFormat::Physical,
    }
}

/// Strip the ="..." quoting that Goodreads uses for ISBN fields.
fn clean_isbn(raw: &str) -> String {
    raw.trim_matches(|c: char| c == '=' || c == '"' || c.is_whitespace())
        .to_string()
}

fn set_provenance(
    conn: &rusqlite::Connection,
    book_id: &Uuid,
    field: &str,
    source: &str,
) -> Result<(), ImportError> {
    conn.execute(
        "INSERT OR IGNORE INTO metadata_provenance (book_id, field_name, source, source_date)
         VALUES (?1, ?2, ?3, ?4)",
        params![book_id.to_string(), field, source, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_test_csv(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("goodreads_export.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "Book Id,Title,Author,Author l-f,Additional Authors,ISBN,ISBN13,My Rating,Average Rating,Publisher,Binding,Number of Pages,Year Published,Original Publication Year,Date Read,Date Added,Bookshelves,Bookshelves with positions,Exclusive Shelf,My Review,Spoiler,Private Notes,Read Count,Owned Copies").unwrap();
        writeln!(f, r#"3,Dune,Frank Herbert,"Herbert, Frank",,="0441172717",="9780441172719",5,4.25,Ace,Paperback,896,2005,1965,2024/03/15,2023/01/10,"sci-fi, classics","sci-fi (#1), classics (#2)",read,"Amazing book about desert planets",,,1,1"#).unwrap();
        writeln!(f, r#"6,Neuromancer,William Gibson,"Gibson, William",,="0441569595",="9780441569595",4,3.89,Ace,Kindle Edition,271,2000,1984,,2024/02/20,cyberpunk,,to-read,,,,0,0"#).unwrap();
        writeln!(f, r#"100,Project Hail Mary,Andy Weir,"Weir, Andy",,="0593135202",="9780593135204",0,4.52,Ballantine Books,Hardcover,496,2021,2021,2024/06/01,2024/05/01,,,currently-reading,,,,0,0"#).unwrap();
        path
    }

    #[test]
    fn import_goodreads_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_test_csv(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let report = import_goodreads(
            &db,
            &csv_path,
            &GoodreadsImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        assert_eq!(report.total_rows, 3);
        assert_eq!(report.imported, 3);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.errors, 0);

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        assert_eq!(books.len(), 3);

        // Check Dune
        let dune = books.iter().find(|b| b.title == "Dune").unwrap();
        assert_eq!(dune.status, ReadingStatus::Read);
        assert_eq!(dune.rating, Some(10)); // 5 * 2
        assert_eq!(dune.page_count, Some(896));
        assert_eq!(dune.format, BookFormat::Physical);

        // Check Neuromancer is ebook
        let neuro = books.iter().find(|b| b.title == "Neuromancer").unwrap();
        assert_eq!(neuro.status, ReadingStatus::WantToRead);
        assert_eq!(neuro.format, BookFormat::Ebook);
        assert_eq!(neuro.rating, Some(8)); // Goodreads 4 * 2 = 8

        // Check currently-reading
        let phm = books
            .iter()
            .find(|b| b.title == "Project Hail Mary")
            .unwrap();
        assert_eq!(phm.status, ReadingStatus::Reading);
    }

    #[test]
    fn import_goodreads_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_test_csv(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let r1 = import_goodreads(
            &db,
            &csv_path,
            &GoodreadsImportOptions { dry_run: false },
            None,
        )
        .unwrap();
        assert_eq!(r1.imported, 3);

        let r2 = import_goodreads(
            &db,
            &csv_path,
            &GoodreadsImportOptions { dry_run: false },
            None,
        )
        .unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.skipped, 3);

        let repo = BookRepository::new(&db);
        assert_eq!(repo.list_books().unwrap().len(), 3);
    }

    #[test]
    fn import_goodreads_dry_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_test_csv(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let report = import_goodreads(
            &db,
            &csv_path,
            &GoodreadsImportOptions { dry_run: true },
            None,
        )
        .unwrap();
        assert_eq!(report.imported, 3);

        let repo = BookRepository::new(&db);
        assert_eq!(repo.list_books().unwrap().len(), 0); // Nothing written
    }

    #[test]
    fn import_goodreads_undo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_test_csv(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let report = import_goodreads(
            &db,
            &csv_path,
            &GoodreadsImportOptions { dry_run: false },
            None,
        )
        .unwrap();
        assert_eq!(report.imported, 3);
        let import_id = report.import_id.unwrap();

        let repo = BookRepository::new(&db);
        assert_eq!(repo.list_books().unwrap().len(), 3);

        let undone = undo_import(&db, &import_id).unwrap();
        assert_eq!(undone, 3);
        assert_eq!(repo.list_books().unwrap().len(), 0);
    }

    #[test]
    fn import_goodreads_authors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_test_csv(tmp.path());
        let db = Database::open_in_memory().unwrap();

        import_goodreads(
            &db,
            &csv_path,
            &GoodreadsImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        let dune = books.iter().find(|b| b.title == "Dune").unwrap();
        let authors = repo.get_book_authors(&dune.id).unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].0.name, "Frank Herbert");
    }

    #[test]
    fn isbn_cleaning() {
        assert_eq!(clean_isbn(r#"="0441172717""#), "0441172717");
        assert_eq!(clean_isbn(r#"="9780441172719""#), "9780441172719");
        assert_eq!(clean_isbn("  9780441172719  "), "9780441172719");
    }
}
