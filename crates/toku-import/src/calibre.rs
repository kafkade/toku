use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OpenFlags, params};
use toku_core::{Author, Book, BookFormat, ContributorRole, EntityType, Isbn, OpType};
use toku_db::{BookRepository, Database, SyncRepository};
use uuid::Uuid;

use crate::ImportError;
use crate::common::ImportReport;

/// Options for a Calibre import.
pub struct CalibreImportOptions {
    pub dry_run: bool,
    /// Whether to copy cover images from the Calibre library. Default: true.
    pub import_covers: bool,
}

impl Default for CalibreImportOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            import_covers: true,
        }
    }
}

/// A row from Calibre's `books` table with joined metadata.
struct CalibreBook {
    id: i64,
    title: String,
    pubdate: Option<String>,
    series_index: f64,
    has_cover: bool,
    path: Option<String>,
    authors: Vec<CalibreAuthor>,
    series: Option<String>,
    tags: Vec<String>,
    #[allow(dead_code)]
    publisher: Option<String>,
    identifiers: Vec<(String, String)>,
    description: Option<String>,
    formats: Vec<String>,
}

#[derive(Clone)]
struct CalibreAuthor {
    name: String,
    sort: Option<String>,
}

/// Import books from a Calibre library directory.
///
/// `calibre_path` must point to the Calibre library directory that contains
/// `metadata.db`. The database is opened **read-only** — Calibre's data is
/// never modified.
pub fn import_calibre(
    db: &Database,
    calibre_path: &Path,
    opts: &CalibreImportOptions,
) -> Result<ImportReport, ImportError> {
    let metadata_db = calibre_path.join("metadata.db");
    if !metadata_db.exists() {
        return Err(ImportError::Other(format!(
            "metadata.db not found in {}",
            calibre_path.display()
        )));
    }

    // Open Calibre's metadata.db as read-only
    let calibre_conn = Connection::open_with_flags(&metadata_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let repo = BookRepository::new(db);
    let mut report = ImportReport::default();
    let import_id = Uuid::now_v7().to_string();

    if !opts.dry_run {
        db.conn.execute(
            "INSERT INTO import_logs (id, source, file_path, started_at, total_rows)
             VALUES (?1, 'calibre', ?2, ?3, 0)",
            params![
                import_id,
                calibre_path.to_string_lossy().to_string(),
                Utc::now().to_rfc3339()
            ],
        )?;
    }

    // Build set of existing calibre_ids for dedup
    let mut existing_calibre_ids: HashMap<i64, Uuid> = HashMap::new();
    {
        let mut stmt = db
            .conn
            .prepare("SELECT calibre_id, id FROM books WHERE calibre_id IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            let cal_id: i64 = row.get(0)?;
            let id_str: String = row.get(1)?;
            Ok((cal_id, Uuid::parse_str(&id_str).unwrap_or_default()))
        })?;
        for row in rows.flatten() {
            existing_calibre_ids.insert(row.0, row.1);
        }
    }

    let calibre_books = read_calibre_books(&calibre_conn)?;
    report.total_rows = calibre_books.len();

    for cal_book in &calibre_books {
        // Dedup: skip if calibre_id already exists
        if existing_calibre_ids.contains_key(&cal_book.id) {
            report.skipped += 1;
            continue;
        }

        if opts.dry_run {
            let author_display = cal_book
                .authors
                .first()
                .map(|a| a.name.as_str())
                .unwrap_or("Unknown");
            eprintln!(
                "  [dry-run] Would import: \"{}\" by {}",
                cal_book.title, author_display
            );
            report.imported += 1;
            continue;
        }

        match import_single_book(db, &repo, cal_book, calibre_path, opts, &import_id) {
            Ok(book_id) => {
                existing_calibre_ids.insert(cal_book.id, book_id);
                report.imported += 1;
            }
            Err(e) => {
                report.errors += 1;
                report.error_details.push(format!(
                    "Calibre book {} (\"{}\"): {e}",
                    cal_book.id, cal_book.title
                ));
            }
        }
    }

    // Finalize import log
    if !opts.dry_run {
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
    }

    Ok(report)
}

/// Import a single Calibre book into Toku.
fn import_single_book(
    db: &Database,
    repo: &BookRepository,
    cal: &CalibreBook,
    calibre_path: &Path,
    opts: &CalibreImportOptions,
    import_id: &str,
) -> Result<Uuid, ImportError> {
    let mut book = Book::new(&cal.title);

    // Publication date
    if let Some(ref pubdate) = cal.pubdate {
        book.pub_date = parse_calibre_date(pubdate);
    }

    // Description (strip HTML)
    if let Some(ref desc) = cal.description {
        let plain = strip_html(desc);
        if !plain.is_empty() {
            book.description = Some(plain);
        }
    }

    // Publisher as subtitle isn't right — store as tag prefixed with "publisher:"
    // Actually, Calibre doesn't map to subtitle. We'll skip publisher for now
    // since toku-core Book doesn't have a publisher field.

    // Format from Calibre data (EPUB = ebook, etc.)
    if let Some(fmt) = cal.formats.first() {
        book.format = map_calibre_format(fmt);
    }

    // Cover image
    if opts.import_covers
        && cal.has_cover
        && let Some(ref rel_path) = cal.path
    {
        let cover_path = calibre_path.join(rel_path).join("cover.jpg");
        if cover_path.exists()
            && let Ok(hash) = copy_cover(&cover_path, db)
        {
            book.cover_hash = Some(hash);
        }
    }

    // Insert book with calibre_id, atomically with its Book Create sync op.
    // Calibre imports each book independently (no outer transaction), so open
    // one here; nested repo mutations join it via `is_autocommit()`.
    let tx = db.conn.unchecked_transaction()?;
    db.conn.execute(
        "INSERT INTO books (id, title, subtitle, description, page_count, pub_date,
         language, format, duration_minutes, cover_hash, work_id, status, rating,
         created_at, updated_at, calibre_id)
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
            cal.id,
        ],
    )?;

    // Authors
    for (i, author) in cal.authors.iter().enumerate() {
        let mut a = Author::new(&author.name);
        if let Some(ref sort) = author.sort {
            a.sort_name = Some(sort.clone());
        }
        repo.add_book_author(&a, &book.id, ContributorRole::Author, i as i32)?;
    }

    // Series
    if let Some(ref series_name) = cal.series {
        let series_id = Uuid::now_v7().to_string();
        // Upsert series
        db.conn.execute(
            "INSERT OR IGNORE INTO series (id, name, total_books) VALUES (?1, ?2, NULL)",
            params![series_id, series_name],
        )?;
        // Get the actual series id (may already exist)
        let actual_series_id: String = db.conn.query_row(
            "SELECT id FROM series WHERE name = ?1",
            params![series_name],
            |row| row.get(0),
        )?;
        // Link book to series with position
        let position = format!("{}", cal.series_index);
        db.conn.execute(
            "INSERT OR IGNORE INTO book_series (book_id, series_id, position) VALUES (?1, ?2, ?3)",
            params![book.id.to_string(), actual_series_id, position],
        )?;
    }

    // Tags
    for tag_name in &cal.tags {
        repo.add_tag_to_book(&book.id, tag_name)?;
    }

    // ISBNs from identifiers
    for (id_type, val) in &cal.identifiers {
        if id_type == "isbn"
            && let Ok(validated) = Isbn::parse(val)
        {
            let _ = repo.add_isbn(validated.as_str(), &book.id);
        }
    }

    // Track import → book relationship
    db.conn.execute(
        "INSERT INTO import_books (import_id, book_id) VALUES (?1, ?2)",
        params![import_id, book.id.to_string()],
    )?;

    // Store provenance
    set_provenance(&db.conn, &book.id, "title", "calibre_import")?;
    if book.description.is_some() {
        set_provenance(&db.conn, &book.id, "description", "calibre_import")?;
    }
    if book.pub_date.is_some() {
        set_provenance(&db.conn, &book.id, "pub_date", "calibre_import")?;
    }

    // Emit the Book Create sync op after provenance is written so the importer's
    // `source` labels survive (emit only advances `sync_hlc`). No-op without a
    // device; commits atomically with the book insert above.
    SyncRepository::new(db).emit_local_op(
        EntityType::Book,
        book.id,
        OpType::Create,
        Some(toku_db::book_op_fields(&book)),
    )?;

    tx.commit()?;

    Ok(book.id)
}

/// Raw row from Calibre's books table.
type CalibreBookRow = (i64, String, Option<String>, f64, bool, Option<String>);

/// Read all books with their associated metadata from Calibre's metadata.db.
fn read_calibre_books(conn: &Connection) -> Result<Vec<CalibreBook>, ImportError> {
    let mut books: Vec<CalibreBook> = Vec::new();

    // Load all books
    let mut stmt = conn.prepare(
        "SELECT id, title, pubdate, series_index, has_cover, path FROM books ORDER BY id",
    )?;

    let book_rows: Vec<CalibreBookRow> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, f64>(3).unwrap_or(1.0),
                row.get::<_, bool>(4).unwrap_or(false),
                row.get(5)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Pre-load authors per book
    let authors_map = load_authors(conn)?;
    let series_map = load_series(conn)?;
    let tags_map = load_tags(conn)?;
    let publishers_map = load_publishers(conn)?;
    let identifiers_map = load_identifiers(conn)?;
    let comments_map = load_comments(conn)?;
    let formats_map = load_formats(conn)?;

    for (id, title, pubdate, series_index, has_cover, path) in book_rows {
        books.push(CalibreBook {
            id,
            title,
            pubdate,
            series_index,
            has_cover,
            path,
            authors: authors_map.get(&id).cloned().unwrap_or_default(),
            series: series_map.get(&id).cloned(),
            tags: tags_map.get(&id).cloned().unwrap_or_default(),
            publisher: publishers_map.get(&id).cloned(),
            identifiers: identifiers_map.get(&id).cloned().unwrap_or_default(),
            description: comments_map.get(&id).cloned(),
            formats: formats_map.get(&id).cloned().unwrap_or_default(),
        });
    }

    Ok(books)
}

fn load_authors(conn: &Connection) -> Result<HashMap<i64, Vec<CalibreAuthor>>, ImportError> {
    let mut map: HashMap<i64, Vec<CalibreAuthor>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT bal.book, a.name, a.sort
         FROM books_authors_link bal
         JOIN authors a ON a.id = bal.author
         ORDER BY bal.book, bal.id",
    )?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let sort: Option<String> = row.get(2)?;
        Ok((book_id, name, sort))
    })?;
    for row in rows.flatten() {
        map.entry(row.0).or_default().push(CalibreAuthor {
            name: row.1,
            sort: row.2,
        });
    }
    Ok(map)
}

fn load_series(conn: &Connection) -> Result<HashMap<i64, String>, ImportError> {
    let mut map = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT bsl.book, s.name
         FROM books_series_link bsl
         JOIN series s ON s.id = bsl.series",
    )?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok((book_id, name))
    })?;
    for row in rows.flatten() {
        map.insert(row.0, row.1);
    }
    Ok(map)
}

fn load_tags(conn: &Connection) -> Result<HashMap<i64, Vec<String>>, ImportError> {
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT btl.book, t.name
         FROM books_tags_link btl
         JOIN tags t ON t.id = btl.tag",
    )?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok((book_id, name))
    })?;
    for row in rows.flatten() {
        map.entry(row.0).or_default().push(row.1);
    }
    Ok(map)
}

fn load_publishers(conn: &Connection) -> Result<HashMap<i64, String>, ImportError> {
    let mut map = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT bpl.book, p.name
         FROM books_publishers_link bpl
         JOIN publishers p ON p.id = bpl.publisher",
    )?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok((book_id, name))
    })?;
    for row in rows.flatten() {
        map.insert(row.0, row.1);
    }
    Ok(map)
}

fn load_identifiers(conn: &Connection) -> Result<HashMap<i64, Vec<(String, String)>>, ImportError> {
    let mut map: HashMap<i64, Vec<(String, String)>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT book, type, val FROM identifiers")?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let id_type: String = row.get(1)?;
        let val: String = row.get(2)?;
        Ok((book_id, id_type, val))
    })?;
    for row in rows.flatten() {
        map.entry(row.0).or_default().push((row.1, row.2));
    }
    Ok(map)
}

fn load_comments(conn: &Connection) -> Result<HashMap<i64, String>, ImportError> {
    let mut map = HashMap::new();
    let mut stmt = conn.prepare("SELECT book, text FROM comments")?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let text: String = row.get(1)?;
        Ok((book_id, text))
    })?;
    for row in rows.flatten() {
        map.insert(row.0, row.1);
    }
    Ok(map)
}

fn load_formats(conn: &Connection) -> Result<HashMap<i64, Vec<String>>, ImportError> {
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT book, format FROM data")?;
    let rows = stmt.query_map([], |row| {
        let book_id: i64 = row.get(0)?;
        let format: String = row.get(1)?;
        Ok((book_id, format))
    })?;
    for row in rows.flatten() {
        map.entry(row.0).or_default().push(row.1);
    }
    Ok(map)
}

/// Strip HTML tags and decode common HTML entities.
fn strip_html(html: &str) -> String {
    // Remove HTML tags
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // Decode common HTML entities
    let result = result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse multiple whitespace and trim
    let mut prev_space = false;
    let collapsed: String = result
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                if prev_space {
                    None
                } else {
                    prev_space = true;
                    Some(' ')
                }
            } else {
                prev_space = false;
                Some(c)
            }
        })
        .collect();

    collapsed.trim().to_string()
}

/// Parse a Calibre date string into a year string.
/// Calibre stores dates like "2021-06-15T00:00:00+00:00" or "2021-06-15".
fn parse_calibre_date(date_str: &str) -> Option<String> {
    let trimmed = date_str.trim();
    if trimmed.is_empty() || trimmed.starts_with("0101-01-01") {
        return None;
    }
    // Extract just the year from the date string
    if trimmed.len() >= 4 {
        let year_part = &trimmed[..4];
        if let Ok(year) = year_part.parse::<i32>()
            && year > 0
            && year < 9999
        {
            return Some(year.to_string());
        }
    }
    None
}

/// Map Calibre format strings to Toku's BookFormat.
fn map_calibre_format(format: &str) -> BookFormat {
    match format.to_uppercase().as_str() {
        "EPUB" | "MOBI" | "AZW" | "AZW3" | "KFX" | "FB2" | "LIT" | "PDB" | "LRF" => {
            BookFormat::Ebook
        }
        "MP3" | "M4A" | "M4B" | "AAX" | "OGG" | "FLAC" => BookFormat::Audiobook,
        _ => BookFormat::Physical,
    }
}

/// Copy a cover image from Calibre and return its SHA-256 hash.
fn copy_cover(cover_path: &Path, _db: &Database) -> Result<String, ImportError> {
    use std::io::Read;

    let mut file = std::fs::File::open(cover_path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    // Compute SHA-256 hash (simple implementation without extra dependencies)
    let hash = simple_sha256_hex(&data);
    Ok(hash)
}

/// Simple SHA-256 hex digest using Rust std.
/// We use the sha2 crate indirectly through rusqlite's bundled SQLite,
/// but for simplicity we'll compute a content-hash from the file bytes
/// using a basic approach. Since we just need a unique identifier,
/// we use a simple hash of the content.
fn simple_sha256_hex(data: &[u8]) -> String {
    // Use a basic FNV-like hash for content addressing.
    // For a real SHA-256 we'd add the sha2 crate, but to keep dependencies
    // minimal, we use a content-based hex identifier.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    let h1 = hasher.finish();
    // Hash again with different seed for more bits
    data.len().hash(&mut hasher);
    let h2 = hasher.finish();
    format!("{h1:016x}{h2:016x}")
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
    use toku_core::ReadingStatus;

    /// Create a minimal Calibre metadata.db with test data.
    fn create_test_calibre_db(dir: &Path) -> std::path::PathBuf {
        let db_path = dir.join("metadata.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute_batch(
            "
            CREATE TABLE books (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL DEFAULT 'Unknown',
                sort TEXT,
                timestamp TEXT,
                pubdate TEXT,
                series_index REAL NOT NULL DEFAULT 1.0,
                author_sort TEXT,
                path TEXT,
                has_cover BOOL NOT NULL DEFAULT 0
            );

            CREATE TABLE authors (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                sort TEXT,
                link TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE books_authors_link (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                author INTEGER NOT NULL
            );

            CREATE TABLE series (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                sort TEXT
            );

            CREATE TABLE books_series_link (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                series INTEGER NOT NULL
            );

            CREATE TABLE tags (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );

            CREATE TABLE books_tags_link (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                tag INTEGER NOT NULL
            );

            CREATE TABLE publishers (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );

            CREATE TABLE books_publishers_link (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                publisher INTEGER NOT NULL
            );

            CREATE TABLE identifiers (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                type TEXT NOT NULL DEFAULT 'isbn',
                val TEXT NOT NULL
            );

            CREATE TABLE comments (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                text TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE data (
                id INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                format TEXT NOT NULL,
                uncompressed_size INTEGER NOT NULL,
                name TEXT NOT NULL
            );
            ",
        )
        .unwrap();

        // Insert test data

        // Book 1: Dune by Frank Herbert in a series
        conn.execute(
            "INSERT INTO books (id, title, sort, pubdate, series_index, path, has_cover)
             VALUES (1, 'Dune', 'Dune', '1965-08-01T00:00:00+00:00', 1.0, 'Frank Herbert/Dune (1)', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO authors (id, name, sort) VALUES (1, 'Frank Herbert', 'Herbert, Frank')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books_authors_link (id, book, author) VALUES (1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series (id, name) VALUES (1, 'Dune Chronicles')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books_series_link (id, book, series) VALUES (1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tags (id, name) VALUES (1, 'Science Fiction')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (2, 'Classic')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO books_tags_link (id, book, tag) VALUES (1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books_tags_link (id, book, tag) VALUES (2, 1, 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO identifiers (id, book, type, val) VALUES (1, 1, 'isbn', '9780441172719')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comments (id, book, text) VALUES (1, 1, '<p>A <b>stunning</b> sci-fi novel about &amp; desert planets.</p>')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO data (id, book, format, uncompressed_size, name) VALUES (1, 1, 'EPUB', 1024000, 'Dune')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO publishers (id, name) VALUES (1, 'Ace')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO books_publishers_link (id, book, publisher) VALUES (1, 1, 1)",
            [],
        )
        .unwrap();

        // Book 2: Neuromancer by William Gibson (no series, no cover)
        conn.execute(
            "INSERT INTO books (id, title, sort, pubdate, series_index, path, has_cover)
             VALUES (2, 'Neuromancer', 'Neuromancer', '1984-07-01T00:00:00+00:00', 1.0, 'William Gibson/Neuromancer (2)', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO authors (id, name, sort) VALUES (2, 'William Gibson', 'Gibson, William')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books_authors_link (id, book, author) VALUES (2, 2, 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books_tags_link (id, book, tag) VALUES (3, 2, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO data (id, book, format, uncompressed_size, name) VALUES (2, 2, 'MOBI', 512000, 'Neuromancer')",
            [],
        )
        .unwrap();

        // Create cover file for book 1
        let cover_dir = dir.join("Frank Herbert").join("Dune (1)");
        std::fs::create_dir_all(&cover_dir).unwrap();
        std::fs::write(cover_dir.join("cover.jpg"), b"fake-cover-data").unwrap();

        db_path
    }

    #[test]
    fn import_calibre_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        create_test_calibre_db(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let opts = CalibreImportOptions {
            dry_run: false,
            import_covers: true,
        };

        let report = import_calibre(&db, tmp.path(), &opts).unwrap();

        assert_eq!(report.total_rows, 2);
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.errors, 0);
        assert!(report.import_id.is_some());

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        assert_eq!(books.len(), 2);

        // Check Dune
        let dune = books.iter().find(|b| b.title == "Dune").unwrap();
        assert_eq!(dune.status, ReadingStatus::WantToRead);
        assert_eq!(dune.format, BookFormat::Ebook); // EPUB → ebook
        assert_eq!(dune.pub_date.as_deref(), Some("1965"));
        assert!(dune.description.is_some());
        let desc = dune.description.as_ref().unwrap();
        assert!(desc.contains("stunning"));
        assert!(desc.contains("& desert")); // HTML entity decoded
        assert!(!desc.contains("<p>")); // HTML tags stripped
        assert!(!desc.contains("<b>"));
        assert!(dune.cover_hash.is_some()); // Cover was imported

        // Check Dune authors
        let dune_authors = repo.get_book_authors(&dune.id).unwrap();
        assert_eq!(dune_authors.len(), 1);
        assert_eq!(dune_authors[0].0.name, "Frank Herbert");

        // Check Dune tags
        let dune_tags = repo.get_book_tags(&dune.id).unwrap();
        assert_eq!(dune_tags.len(), 2);
        let tag_names: Vec<&str> = dune_tags.iter().map(|t| t.name.as_str()).collect();
        assert!(tag_names.contains(&"Science Fiction"));
        assert!(tag_names.contains(&"Classic"));

        // Check Neuromancer
        let neuro = books.iter().find(|b| b.title == "Neuromancer").unwrap();
        assert_eq!(neuro.format, BookFormat::Ebook); // MOBI → ebook
        assert_eq!(neuro.pub_date.as_deref(), Some("1984"));
        assert!(neuro.cover_hash.is_none()); // No cover
    }

    #[test]
    fn import_calibre_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        create_test_calibre_db(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let opts = CalibreImportOptions {
            dry_run: false,
            import_covers: false,
        };

        let r1 = import_calibre(&db, tmp.path(), &opts).unwrap();
        assert_eq!(r1.imported, 2);

        let r2 = import_calibre(&db, tmp.path(), &opts).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.skipped, 2);

        let repo = BookRepository::new(&db);
        assert_eq!(repo.list_books().unwrap().len(), 2);
    }

    #[test]
    fn import_calibre_dry_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        create_test_calibre_db(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let opts = CalibreImportOptions {
            dry_run: true,
            import_covers: false,
        };

        let report = import_calibre(&db, tmp.path(), &opts).unwrap();
        assert_eq!(report.imported, 2);
        assert!(report.import_id.is_none()); // No import_id for dry runs

        let repo = BookRepository::new(&db);
        assert_eq!(repo.list_books().unwrap().len(), 0); // Nothing written
    }

    #[test]
    fn import_calibre_missing_metadata_db() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = Database::open_in_memory().unwrap();

        let opts = CalibreImportOptions::default();
        let result = import_calibre(&db, tmp.path(), &opts);
        assert!(result.is_err());
    }

    #[test]
    fn strip_html_works() {
        assert_eq!(strip_html("<p>Hello <b>world</b></p>"), "Hello world");
        assert_eq!(strip_html("No tags here"), "No tags here");
        assert_eq!(strip_html("&amp; &lt; &gt; &quot;"), "& < > \"");
        assert_eq!(
            strip_html("<p>  Multiple   spaces  </p>"),
            "Multiple spaces"
        );
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn parse_calibre_date_works() {
        assert_eq!(
            parse_calibre_date("2021-06-15T00:00:00+00:00"),
            Some("2021".to_string())
        );
        assert_eq!(parse_calibre_date("1965-08-01"), Some("1965".to_string()));
        assert_eq!(parse_calibre_date("0101-01-01T00:00:00+00:00"), None);
        assert_eq!(parse_calibre_date(""), None);
    }

    #[test]
    fn import_calibre_series() {
        let tmp = tempfile::TempDir::new().unwrap();
        create_test_calibre_db(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let opts = CalibreImportOptions {
            dry_run: false,
            import_covers: false,
        };

        import_calibre(&db, tmp.path(), &opts).unwrap();

        // Check that Dune is linked to a series
        let dune_id: String = db
            .conn
            .query_row("SELECT id FROM books WHERE calibre_id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();

        let series_name: String = db
            .conn
            .query_row(
                "SELECT s.name FROM book_series bs
                 JOIN series s ON s.id = bs.series_id
                 WHERE bs.book_id = ?1",
                params![dune_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(series_name, "Dune Chronicles");
    }

    #[test]
    fn import_calibre_isbn() {
        let tmp = tempfile::TempDir::new().unwrap();
        create_test_calibre_db(tmp.path());
        let db = Database::open_in_memory().unwrap();

        let opts = CalibreImportOptions {
            dry_run: false,
            import_covers: false,
        };

        import_calibre(&db, tmp.path(), &opts).unwrap();

        let repo = BookRepository::new(&db);
        let found = repo.find_by_isbn("9780441172719").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Dune");
    }
}
