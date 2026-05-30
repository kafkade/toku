use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use csv::ReaderBuilder;
use rusqlite::params;
use toku_core::{
    Author, Book, BookFormat, ContributorRole, Isbn, ReadingSession, ReadingStatus, TagType,
};
use toku_db::{BookRepository, Database};
use uuid::Uuid;

use crate::ImportError;
use crate::common::{
    ImportObserver, ImportReport, MAX_REPORT_SAMPLES, RowOutcome, RowSummary, count_csv_rows,
    emit_event, set_provenance,
};

/// Options for a StoryGraph import.
pub struct StorygraphImportOptions {
    pub dry_run: bool,
}

/// Import books from a StoryGraph CSV export.
///
/// If an `observer` is provided, it will be called with progress events for
/// each row processed, enabling progress bars and live feedback.
pub fn import_storygraph(
    db: &Database,
    csv_path: &Path,
    opts: &StorygraphImportOptions,
    observer: Option<&mut dyn ImportObserver>,
) -> Result<ImportReport, ImportError> {
    let total_rows = count_csv_rows(csv_path)?;
    import_storygraph_inner(db, csv_path, opts, observer, total_rows)
}

fn import_storygraph_inner(
    db: &Database,
    csv_path: &Path,
    opts: &StorygraphImportOptions,
    mut observer: Option<&mut dyn ImportObserver>,
    total_rows: usize,
) -> Result<ImportReport, ImportError> {
    let repo = BookRepository::new(db);
    let mut report = ImportReport::default();

    let import_id = Uuid::now_v7().to_string();

    if !opts.dry_run {
        db.conn.execute_batch("BEGIN IMMEDIATE")?;

        db.conn.execute(
            "INSERT INTO import_logs (id, source, file_path, started_at, total_rows)
             VALUES (?1, 'storygraph', ?2, ?3, 0)",
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
    opts: &StorygraphImportOptions,
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

    // Detect: StoryGraph has Moods/Pace columns that Goodreads doesn't
    let has_moods = headers.iter().any(|h| h == "Moods");
    let has_pace = headers.iter().any(|h| h == "Pace");
    if !has_moods && !has_pace {
        return Err(ImportError::Other(
            "CSV does not contain 'Moods' or 'Pace' columns — is this a StoryGraph export?"
                .to_string(),
        ));
    }

    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| h == name) };

    let col_title = col("Title");
    let col_authors = col("Authors");
    let col_contributors = col("Contributors");
    let col_isbn_uid = col("ISBN/UID");
    let col_format = col("Format");
    let col_status = col("Read Status");
    let col_date_added = col("Date Added");
    let col_dates_read = col("Dates Read");
    let col_moods = col("Moods");
    let col_pace = col("Pace");
    let col_star_rating = col("Star Rating");
    let col_content_warnings = col("Content Warnings");
    let col_tags = col("Tags");
    let col_review = col("Review");
    let col_char_plot = col("Character- or Plot-Driven?");
    let col_strong_dev = col("Strong Character Development?");
    let col_loveable = col("Loveable Characters?");
    let col_diverse = col("Diverse Characters?");
    let col_flawed = col("Flawed Characters?");

    if col_title.is_none() {
        return Err(ImportError::Other(
            "CSV does not contain a 'Title' column".to_string(),
        ));
    }

    // Build dedup maps: ISBN → book_id, and normalized(title|author) → book_id
    let mut isbn_map: HashMap<String, Uuid> = HashMap::new();
    {
        let mut stmt = db.conn.prepare("SELECT isbn, book_id FROM isbns")?;
        let rows = stmt.query_map([], |row| {
            let isbn: String = row.get(0)?;
            let id_str: String = row.get(1)?;
            Ok((isbn, Uuid::parse_str(&id_str).unwrap_or_default()))
        })?;
        for row in rows.flatten() {
            isbn_map.insert(row.0, row.1);
        }
    }

    let mut title_author_map: HashMap<String, Uuid> = HashMap::new();
    {
        let mut stmt = db.conn.prepare(
            "SELECT b.id, LOWER(TRIM(b.title)), COALESCE(LOWER(TRIM(a.name)), '')
             FROM books b
             LEFT JOIN book_authors ba ON ba.book_id = b.id AND ba.position = 0
             LEFT JOIN authors a ON a.id = ba.author_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let title: String = row.get(1)?;
            let author: String = row.get(2)?;
            Ok((Uuid::parse_str(&id_str).unwrap_or_default(), title, author))
        })?;
        for row in rows.flatten() {
            let key = format!("{}|{}", row.1, row.2);
            title_author_map.insert(key, row.0);
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

        let authors_raw = get(col_authors).unwrap_or_default();
        let primary_author = authors_raw
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();

        let status = get(col_status)
            .as_deref()
            .map(map_storygraph_status)
            .unwrap_or(ReadingStatus::WantToRead);
        let status_str = status.as_str().to_string();

        // Dedup: check ISBN first, then title+author
        let isbn_uid = get(col_isbn_uid);
        let parsed_isbn = isbn_uid.as_deref().and_then(detect_isbn);

        let existing_book_id = parsed_isbn
            .as_ref()
            .and_then(|isbn| isbn_map.get(isbn))
            .copied()
            .or_else(|| {
                let key = format!(
                    "{}|{}",
                    title.to_lowercase().trim(),
                    primary_author.to_lowercase().trim()
                );
                title_author_map.get(&key).copied()
            });

        if let Some(existing_id) = existing_book_id {
            // Apply typed tags to existing book (moods, pace, CW) but don't overwrite
            if !opts.dry_run {
                apply_typed_tags(
                    repo,
                    &existing_id,
                    &get,
                    col_moods,
                    col_pace,
                    col_content_warnings,
                )?;
                apply_general_tags(repo, &existing_id, &get, col_tags)?;
                apply_character_tags(
                    repo,
                    &existing_id,
                    &get,
                    col_char_plot,
                    col_strong_dev,
                    col_loveable,
                    col_diverse,
                    col_flawed,
                )?;
            }

            report.updated += 1;
            if report.updated_samples.len() < MAX_REPORT_SAMPLES {
                report.updated_samples.push(RowSummary {
                    title: title.clone(),
                    author: primary_author.clone(),
                    status: status_str.clone(),
                });
            }
            emit_event(
                observer,
                report.total_rows,
                total_rows,
                &title,
                &primary_author,
                &status_str,
                RowOutcome::Updated,
            )?;
            continue;
        }

        if opts.dry_run {
            report.imported += 1;
            *report.status_counts.entry(status_str.clone()).or_insert(0) += 1;
            if report.imported_samples.len() < MAX_REPORT_SAMPLES {
                report.imported_samples.push(RowSummary {
                    title: title.clone(),
                    author: primary_author.clone(),
                    status: status_str.clone(),
                });
            }
            emit_event(
                observer,
                report.total_rows,
                total_rows,
                &title,
                &primary_author,
                &status_str,
                RowOutcome::Imported,
            )?;
            continue;
        }

        // ── Build the book ──────────────────────────────────────────
        let mut book = Book::new(&title);
        book.status = status;

        // Format
        if let Some(fmt) = get(col_format) {
            book.format = map_storygraph_format(&fmt);
        }

        // Rating: quarter-star → 0-10
        if let Some(rating_str) = get(col_star_rating)
            && let Ok(star) = rating_str.parse::<f64>()
            && (1.0..=5.0).contains(&star)
        {
            book.rating = Some((star * 2.0).round() as i32);
        }

        // Date Added → created_at
        if let Some(date_str) = get(col_date_added)
            && let Some(dt) = parse_storygraph_date(&date_str)
        {
            book.created_at = dt.and_utc();
            book.updated_at = dt.and_utc();
        }

        // Insert the book
        let search_text = format!("{} {}", book.title, primary_author);
        db.conn.execute(
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

        // ISBN
        if let Some(ref isbn) = parsed_isbn {
            let _ = repo.add_isbn(isbn, &book.id);
            isbn_map.insert(isbn.clone(), book.id);
        }

        // Authors
        for (i, author_name) in authors_raw.split(',').enumerate() {
            let name = author_name.trim();
            if !name.is_empty() {
                let a = Author::new(name);
                repo.add_book_author(&a, &book.id, ContributorRole::Author, i as i32)?;
            }
        }

        // Contributors (narrators etc.)
        if let Some(contribs) = get(col_contributors) {
            parse_contributors(&contribs, repo, &book.id)?;
        }

        // Typed tags: moods, pace, content warnings
        apply_typed_tags(
            repo,
            &book.id,
            &get,
            col_moods,
            col_pace,
            col_content_warnings,
        )?;

        // General tags
        apply_general_tags(repo, &book.id, &get, col_tags)?;

        // Character/plot-driven and character questions → general tags
        apply_character_tags(
            repo,
            &book.id,
            &get,
            col_char_plot,
            col_strong_dev,
            col_loveable,
            col_diverse,
            col_flawed,
        )?;

        // Reading sessions from Dates Read (full-precision only)
        if let Some(dates_str) = get(col_dates_read) {
            let sessions = parse_dates_read(&dates_str);
            for (start, end) in &sessions {
                let mut session = ReadingSession::new(book.id);
                session.started_at = start.and_utc();
                if let Some(e) = end {
                    session.finished_at = Some(e.and_utc());
                }
                repo.create_reading_session(&session)?;
            }
        }

        // Review → warn, not stored
        if get(col_review).is_some() && report.warnings.len() < MAX_REPORT_SAMPLES {
            report
                .warnings
                .push(format!("\"{}\": review skipped (no reviews table)", title));
        }

        // Track import → book
        db.conn.execute(
            "INSERT INTO import_books (import_id, book_id) VALUES (?1, ?2)",
            params![import_id, book.id.to_string()],
        )?;

        // Provenance
        set_provenance(&db.conn, &book.id, "title", "storygraph_import")?;
        if book.rating.is_some() {
            set_provenance(&db.conn, &book.id, "rating", "storygraph_import")?;
        }

        // Update dedup maps
        let key = format!(
            "{}|{}",
            title.to_lowercase().trim(),
            primary_author.to_lowercase().trim()
        );
        title_author_map.insert(key, book.id);

        let book_status = book.status.as_str().to_string();
        report.imported += 1;
        *report.status_counts.entry(book_status.clone()).or_insert(0) += 1;
        if report.imported_samples.len() < MAX_REPORT_SAMPLES {
            report.imported_samples.push(RowSummary {
                title: title.clone(),
                author: primary_author.clone(),
                status: book_status.clone(),
            });
        }

        emit_event(
            observer,
            report.total_rows,
            total_rows,
            &title,
            &primary_author,
            &book_status,
            RowOutcome::Imported,
        )?;
    }

    Ok(())
}

// ── Field mapping helpers ───────────────────────────────────────────────

fn map_storygraph_status(status: &str) -> ReadingStatus {
    match status.to_lowercase().as_str() {
        "read" => ReadingStatus::Read,
        "currently-reading" => ReadingStatus::Reading,
        "to-read" => ReadingStatus::WantToRead,
        "did-not-finish" => ReadingStatus::Abandoned,
        "paused" => ReadingStatus::OnHold,
        _ => ReadingStatus::WantToRead,
    }
}

fn map_storygraph_format(fmt: &str) -> BookFormat {
    match fmt.to_lowercase().as_str() {
        "digital" | "ebook" => BookFormat::Ebook,
        "audio" | "audiobook" => BookFormat::Audiobook,
        _ => BookFormat::Physical, // paperback, hardcover
    }
}

/// Detect whether an ISBN/UID field contains a valid ISBN and return the
/// normalized form. ASINs and unknown IDs are ignored for dedup purposes.
fn detect_isbn(raw: &str) -> Option<String> {
    let cleaned = raw.trim();
    if cleaned.is_empty() {
        return None;
    }
    // Try parsing as ISBN (handles both ISBN-10 and ISBN-13)
    if let Ok(isbn) = Isbn::parse(cleaned) {
        return Some(isbn.as_str().to_string());
    }
    None
}

/// Parse a StoryGraph date (YYYY/MM/DD, YYYY/MM, or YYYY).
/// Returns a NaiveDateTime at midnight UTC for fully-specified dates only
/// when used for session creation. For created_at, we use any precision.
fn parse_storygraph_date(date_str: &str) -> Option<chrono::NaiveDateTime> {
    let parts: Vec<&str> = date_str.split('/').collect();
    match parts.len() {
        3 => {
            let y: i32 = parts[0].parse().ok()?;
            let m: u32 = parts[1].parse().ok()?;
            let d: u32 = parts[2].parse().ok()?;
            chrono::NaiveDate::from_ymd_opt(y, m, d).map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        }
        2 => {
            let y: i32 = parts[0].parse().ok()?;
            let m: u32 = parts[1].parse().ok()?;
            chrono::NaiveDate::from_ymd_opt(y, m, 1).map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        }
        1 => {
            let y: i32 = parts[0].parse().ok()?;
            chrono::NaiveDate::from_ymd_opt(y, 1, 1).map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        }
        _ => None,
    }
}

/// Check if a date string has full YYYY/MM/DD precision.
fn is_full_precision(date_str: &str) -> bool {
    date_str.split('/').count() == 3
}

/// Parse the multi-session "Dates Read" field.
/// Returns only sessions with full-precision dates (YYYY/MM/DD).
///
/// Format examples:
///   "2025/12/11-2025/12/14"
///   "2025/07/08-2025/07/08, 2024"
///   "2025/02/13-2025/03/15, 2024/09-2024/09"
fn parse_dates_read(raw: &str) -> Vec<(chrono::NaiveDateTime, Option<chrono::NaiveDateTime>)> {
    let mut sessions = Vec::new();
    for session_str in raw.split(", ") {
        let session_str = session_str.trim();
        if session_str.is_empty() {
            continue;
        }
        let parts: Vec<&str> = session_str.splitn(2, '-').collect();
        match parts.len() {
            2 => {
                if is_full_precision(parts[0])
                    && is_full_precision(parts[1])
                    && let (Some(start), Some(end)) = (
                        parse_storygraph_date(parts[0]),
                        parse_storygraph_date(parts[1]),
                    )
                {
                    sessions.push((start, Some(end)));
                }
            }
            1 if is_full_precision(parts[0]) => {
                if let Some(start) = parse_storygraph_date(parts[0]) {
                    sessions.push((start, None));
                }
            }
            _ => {}
        }
    }
    sessions
}

/// Parse content warnings in "Severity: Type; Severity: Type;" format.
fn parse_content_warnings(raw: &str) -> Vec<String> {
    raw.split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            // Try to extract the type after the colon
            if let Some((_severity, cw_type)) = part.split_once(':') {
                let cw = cw_type.trim().to_lowercase();
                if !cw.is_empty() {
                    return Some(cw);
                }
            }
            // If no colon, use the whole string
            let cw = part.to_lowercase();
            if !cw.is_empty() { Some(cw) } else { None }
        })
        .collect()
}

/// Parse Contributors field: "Name (Role), Name (Role)"
fn parse_contributors(raw: &str, repo: &BookRepository, book_id: &Uuid) -> Result<(), ImportError> {
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Try to parse "Name (Role)" pattern
        let (name, role) = if let Some(paren_start) = entry.rfind('(') {
            let name = entry[..paren_start].trim();
            let role_str = entry[paren_start + 1..]
                .trim_end_matches(')')
                .trim()
                .to_lowercase();
            let role = match role_str.as_str() {
                "narrator" => ContributorRole::Narrator,
                "translator" => ContributorRole::Translator,
                "illustrator" => ContributorRole::Illustrator,
                "editor" => ContributorRole::Editor,
                _ => ContributorRole::Author,
            };
            (name, role)
        } else {
            (entry, ContributorRole::Author)
        };

        if !name.is_empty() {
            let a = Author::new(name);
            // Use position 99 for contributors (after main authors)
            repo.add_book_author(&a, book_id, role, 99)?;
        }
    }
    Ok(())
}

/// Apply mood, pace, and content warning tags to a book.
fn apply_typed_tags(
    repo: &BookRepository,
    book_id: &Uuid,
    get: &dyn Fn(Option<usize>) -> Option<String>,
    col_moods: Option<usize>,
    col_pace: Option<usize>,
    col_content_warnings: Option<usize>,
) -> Result<(), ImportError> {
    // Moods
    if let Some(moods_str) = get(col_moods) {
        for mood in moods_str.split(',') {
            let mood = mood.trim().to_lowercase();
            if !mood.is_empty() {
                repo.add_typed_tag_to_book(book_id, &mood, TagType::Mood)?;
            }
        }
    }

    // Pace (single-value: only set if book doesn't already have one)
    if let Some(pace_str) = get(col_pace) {
        let pace = pace_str.trim().to_lowercase();
        if !pace.is_empty() {
            let existing = repo.get_book_tags_by_type(book_id, TagType::Pace)?;
            if existing.is_empty() {
                repo.add_typed_tag_to_book(book_id, &pace, TagType::Pace)?;
            }
        }
    }

    // Content warnings
    if let Some(cw_str) = get(col_content_warnings) {
        let warnings = parse_content_warnings(&cw_str);
        for cw in &warnings {
            repo.add_typed_tag_to_book(book_id, cw, TagType::ContentWarning)?;
        }
    }

    Ok(())
}

/// Apply general tags (from StoryGraph's Tags column).
fn apply_general_tags(
    repo: &BookRepository,
    book_id: &Uuid,
    get: &dyn Fn(Option<usize>) -> Option<String>,
    col_tags: Option<usize>,
) -> Result<(), ImportError> {
    if let Some(tags_str) = get(col_tags) {
        for tag in tags_str.split(',') {
            let tag = tag.trim().to_string();
            if !tag.is_empty() {
                repo.add_tag_to_book(book_id, &tag)?;
            }
        }
    }
    Ok(())
}

/// Apply character-related StoryGraph questions as general tags.
#[allow(clippy::too_many_arguments)]
fn apply_character_tags(
    repo: &BookRepository,
    book_id: &Uuid,
    get: &dyn Fn(Option<usize>) -> Option<String>,
    col_char_plot: Option<usize>,
    col_strong_dev: Option<usize>,
    col_loveable: Option<usize>,
    col_diverse: Option<usize>,
    col_flawed: Option<usize>,
) -> Result<(), ImportError> {
    if let Some(val) = get(col_char_plot) {
        let tag = match val.to_lowercase().as_str() {
            "character" => Some("character-driven"),
            "plot" => Some("plot-driven"),
            "a mix" => Some("character-and-plot-driven"),
            _ => None,
        };
        if let Some(tag) = tag {
            repo.add_tag_to_book(book_id, tag)?;
        }
    }

    let char_questions = [
        (col_strong_dev, "strong-character-development"),
        (col_loveable, "loveable-characters"),
        (col_diverse, "diverse-characters"),
        (col_flawed, "flawed-characters"),
    ];

    for (col, tag_name) in &char_questions {
        if let Some(val) = get(*col)
            && val.eq_ignore_ascii_case("Yes")
        {
            repo.add_tag_to_book(book_id, tag_name)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_storygraph_csv(dir: &Path, rows: &[&str]) -> std::path::PathBuf {
        let path = dir.join("storygraph_export.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        // Header: 23 columns
        writeln!(
            f,
            "Title,Authors,Contributors,ISBN/UID,Format,Read Status,Date Added,\
             Last Date Read,Dates Read,Read Count,Moods,Pace,\
             Character- or Plot-Driven?,Strong Character Development?,\
             Loveable Characters?,Diverse Characters?,Flawed Characters?,\
             Star Rating,Review,Content Warnings,Content Warning Description,Tags,Owned?"
        )
        .unwrap();
        for row in rows {
            writeln!(f, "{row}").unwrap();
        }
        path
    }

    fn test_csv_basic() -> Vec<&'static str> {
        vec![
            // Dune: read, rated 4.5, has moods+pace, ISBN-13
            r#"Dune,Frank Herbert,,9780441172719,paperback,read,2023/01/10,2024/03/15,"2024/03/01-2024/03/15",1,"adventurous, mysterious",fast,Plot,Yes,No,No,Yes,4.5,,,,sci-fi,Yes"#,
            // Neuromancer: to-read, no rating, digital format
            r#"Neuromancer,William Gibson,,9780441569595,digital,to-read,2024/02/20,,,,,,,,,,,,,,cyberpunk,No"#,
            // Project Hail Mary: currently-reading, DNF example actually
            r#"Project Hail Mary,Andy Weir,,9780593135204,hardcover,did-not-finish,2024/05/01,,,,dark,slow,Character,Yes,Yes,Yes,No,2.0,,,,abandoned-test,No"#,
        ]
    }

    #[test]
    fn import_storygraph_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(tmp.path(), &test_csv_basic());
        let db = Database::open_in_memory().unwrap();

        let report = import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
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
        assert_eq!(dune.rating, Some(9)); // 4.5 * 2 = 9
        assert_eq!(dune.format, BookFormat::Physical);

        // Check moods
        let moods = repo.get_book_tags_by_type(&dune.id, TagType::Mood).unwrap();
        assert_eq!(moods.len(), 2);

        // Check pace
        let pace = repo.get_book_tags_by_type(&dune.id, TagType::Pace).unwrap();
        assert_eq!(pace.len(), 1);
        assert_eq!(pace[0].name, "fast");

        // Check Neuromancer is ebook
        let neuro = books.iter().find(|b| b.title == "Neuromancer").unwrap();
        assert_eq!(neuro.status, ReadingStatus::WantToRead);
        assert_eq!(neuro.format, BookFormat::Ebook);
        assert!(neuro.rating.is_none());

        // Check DNF status
        let phm = books
            .iter()
            .find(|b| b.title == "Project Hail Mary")
            .unwrap();
        assert_eq!(phm.status, ReadingStatus::Abandoned);
        assert_eq!(phm.rating, Some(4)); // 2.0 * 2 = 4
    }

    #[test]
    fn import_storygraph_dry_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(tmp.path(), &test_csv_basic());
        let db = Database::open_in_memory().unwrap();

        let report = import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: true },
            None,
        )
        .unwrap();

        assert_eq!(report.total_rows, 3);
        assert_eq!(report.imported, 3);
        assert!(report.import_id.is_none());

        // No books should be in the database
        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        assert!(books.is_empty());
    }

    #[test]
    fn import_storygraph_idempotent_isbn() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(tmp.path(), &test_csv_basic());
        let db = Database::open_in_memory().unwrap();

        // First import
        let r1 = import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();
        assert_eq!(r1.imported, 3);

        // Second import — same ISBNs should match
        let r2 = import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.updated, 3); // tags applied to existing books

        // Still only 3 books
        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        assert_eq!(books.len(), 3);
    }

    #[test]
    fn import_storygraph_dedup_by_title_author() {
        let tmp = tempfile::TempDir::new().unwrap();
        // First: book without ISBN
        let csv1 = write_storygraph_csv(
            tmp.path(),
            &[r#"Dune,Frank Herbert,,,paperback,read,2023/01/10,,,,,,,,,,,,4.0,,,,No"#],
        );
        let db = Database::open_in_memory().unwrap();

        import_storygraph(
            &db,
            &csv1,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        // Second: same title+author, different file
        let dir2 = tmp.path().join("second");
        std::fs::create_dir_all(&dir2).unwrap();
        let csv2 = write_storygraph_csv(
            &dir2,
            &[r#"Dune,Frank Herbert,,,hardcover,read,2024/01/01,,,,,,,,,,,,5.0,,,,No"#],
        );

        let r2 = import_storygraph(
            &db,
            &csv2,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        assert_eq!(r2.imported, 0);
        assert_eq!(r2.updated, 1);

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        assert_eq!(books.len(), 1); // no duplicate
    }

    #[test]
    fn rating_conversion_quarter_star() {
        // Test boundary cases
        assert_eq!((1.0_f64 * 2.0).round() as i32, 2);
        assert_eq!((1.25_f64 * 2.0).round() as i32, 3); // rounds up
        assert_eq!((1.75_f64 * 2.0).round() as i32, 4); // rounds up
        assert_eq!((2.5_f64 * 2.0).round() as i32, 5);
        assert_eq!((3.75_f64 * 2.0).round() as i32, 8); // rounds up
        assert_eq!((4.25_f64 * 2.0).round() as i32, 9); // rounds up
        assert_eq!((4.5_f64 * 2.0).round() as i32, 9);
        assert_eq!((4.75_f64 * 2.0).round() as i32, 10); // rounds up
        assert_eq!((5.0_f64 * 2.0).round() as i32, 10);
    }

    #[test]
    fn parse_dates_read_full_precision() {
        let sessions = parse_dates_read("2025/12/11-2025/12/14");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].0,
            chrono::NaiveDate::from_ymd_opt(2025, 12, 11)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        );
    }

    #[test]
    fn parse_dates_read_multi_session() {
        let sessions = parse_dates_read("2025/07/08-2025/07/08, 2025/02/13-2025/03/15");
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn parse_dates_read_skips_partial() {
        // Year-only and month-only should be skipped
        let sessions = parse_dates_read("2025/07/08-2025/07/08, 2024");
        assert_eq!(sessions.len(), 1); // only the full-precision one

        let sessions = parse_dates_read("2024/09-2024/09");
        assert!(sessions.is_empty());

        let sessions = parse_dates_read("2024");
        assert!(sessions.is_empty());
    }

    #[test]
    fn parse_content_warnings_format() {
        let cws = parse_content_warnings(
            "Graphic: Sexual content; Moderate: Cancer; Minor: Death of parent;",
        );
        assert_eq!(cws.len(), 3);
        assert_eq!(cws[0], "sexual content");
        assert_eq!(cws[1], "cancer");
        assert_eq!(cws[2], "death of parent");
    }

    #[test]
    fn content_warnings_imported_as_tags() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(
            tmp.path(),
            &[
                r#"Test Book,Test Author,,,paperback,read,2024/01/01,,,,,,,,,,,,,Graphic: Violence; Minor: Death;,,,No"#,
            ],
        );
        let db = Database::open_in_memory().unwrap();

        import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        let cw_tags = repo
            .get_book_tags_by_type(&books[0].id, TagType::ContentWarning)
            .unwrap();
        assert_eq!(cw_tags.len(), 2);
        let names: Vec<&str> = cw_tags.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"violence"));
        assert!(names.contains(&"death"));
    }

    #[test]
    fn contributors_parsed_as_narrator() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(
            tmp.path(),
            &[
                r#"Test Book,Test Author,"Ray Porter (Narrator)",,audio,read,2024/01/01,,,,,,,,,,,,,,,No"#,
            ],
        );
        let db = Database::open_in_memory().unwrap();

        import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        let authors = repo.get_book_authors(&books[0].id).unwrap();
        assert_eq!(authors.len(), 2); // Test Author + Ray Porter
        let narrator = authors.iter().find(|(a, _)| a.name == "Ray Porter");
        assert!(narrator.is_some());
        assert_eq!(narrator.unwrap().1.role, ContributorRole::Narrator);
    }

    #[test]
    fn reading_sessions_created_from_dates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(
            tmp.path(),
            &[
                r#"Dune,Frank Herbert,,,paperback,read,2023/01/10,,"2024/03/01-2024/03/15, 2023/06/10-2023/06/20",2,,,,,,,,,,,,,No"#,
            ],
        );
        let db = Database::open_in_memory().unwrap();

        import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        let sessions = repo.list_reading_sessions().unwrap();
        let book_sessions: Vec<_> = sessions
            .iter()
            .filter(|s| s.book_id == books[0].id)
            .collect();
        assert_eq!(book_sessions.len(), 2);
    }

    #[test]
    fn detect_storygraph_csv_rejects_goodreads() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("goodreads.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "Book Id,Title,Author,ISBN,My Rating,Exclusive Shelf").unwrap();
        writeln!(f, "1,Dune,Frank Herbert,1234567890,5,read").unwrap();

        let result = import_storygraph(
            &Database::open_in_memory().unwrap(),
            &path,
            &StorygraphImportOptions { dry_run: false },
            None,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("StoryGraph export")
        );
    }

    #[test]
    fn character_driven_imported_as_tag() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(
            tmp.path(),
            &[
                r#"Test Book,Author,,,paperback,read,2024/01/01,,,,,,Character,Yes,Yes,No,No,,,,,custom-tag,No"#,
            ],
        );
        let db = Database::open_in_memory().unwrap();

        import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        let tags = repo.get_book_tags(&books[0].id).unwrap();
        let tag_names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        assert!(tag_names.contains(&"character-driven"));
        assert!(tag_names.contains(&"strong-character-development"));
        assert!(tag_names.contains(&"loveable-characters"));
        assert!(tag_names.contains(&"custom-tag"));
    }

    #[test]
    fn review_not_stored_in_description() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(
            tmp.path(),
            &[
                r#"Test Book,Author,,,paperback,read,2024/01/01,,,,,,,,,,,,"<div>Great book!</div>",,,,No"#,
            ],
        );
        let db = Database::open_in_memory().unwrap();

        let report = import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        // Description should remain None
        assert!(books[0].description.is_none());
        // Warning should be recorded
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("review skipped"));
    }

    #[test]
    fn paused_status_maps_to_on_hold() {
        let tmp = tempfile::TempDir::new().unwrap();
        let csv_path = write_storygraph_csv(
            tmp.path(),
            &[r#"Test Book,Author,,,paperback,paused,2024/01/01,,,,,,,,,,,,,,,No"#],
        );
        let db = Database::open_in_memory().unwrap();

        import_storygraph(
            &db,
            &csv_path,
            &StorygraphImportOptions { dry_run: false },
            None,
        )
        .unwrap();

        let repo = BookRepository::new(&db);
        let books = repo.list_books().unwrap();
        assert_eq!(books[0].status, ReadingStatus::OnHold);
    }
}
