use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use toku_core::{
    Author, Book, BookFormat, ContributorRole, CurrentlyReadingInput, Isbn, ProgressType,
    ReadingProgress, ReadingSession, ReadingStatus, TokuConfig, compute_stats,
    parse_duration_to_minutes,
};
use toku_db::{BookRepository, Database};

mod import_ui;
mod tui;

/// Toku — a private, offline-first personal book manager.
#[derive(Parser)]
#[command(name = "toku", version, about)]
struct Cli {
    /// Data directory (default: platform-specific)
    #[arg(long, env = "TOKU_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,

    /// Output format
    #[arg(long, default_value = "table", global = true)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive library browser (TUI) — default when no subcommand given
    Browse,

    /// Add a book to your library
    Add {
        /// Book title (for manual entry)
        #[arg(long, short)]
        title: Option<String>,

        /// Author name (for manual entry)
        #[arg(long, short)]
        author: Option<String>,

        /// ISBN to look up and add
        #[arg(long, short)]
        isbn: Option<String>,

        /// Book format
        #[arg(long, default_value = "physical")]
        book_format: String,
    },

    /// Show details of a book
    Show {
        /// Book title or ID to display
        query: String,
    },

    /// List books in your library
    List {
        /// Filter by reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by shelf name
        #[arg(long)]
        shelf: Option<String>,

        /// Filter by tag name
        #[arg(long)]
        tag: Option<String>,
    },

    /// Search your library
    Search {
        /// Search query
        query: String,

        /// Filter by reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by shelf name
        #[arg(long)]
        shelf: Option<String>,

        /// Filter by tag name
        #[arg(long)]
        tag: Option<String>,
    },

    /// Import books from external sources
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },

    /// Show or edit configuration
    Config {
        /// Print the config file path
        #[arg(long)]
        path: bool,

        /// Open the config file in your editor
        #[arg(long)]
        edit: bool,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },

    /// Manage reading status (start, finish, abandon, hold, resume)
    Reading {
        #[command(subcommand)]
        action: ReadingAction,
    },

    /// Manage shelves for organizing books
    Shelf {
        #[command(subcommand)]
        action: ShelfAction,
    },

    /// Manage tags for categorizing books
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },

    /// Export your library (csv, json, markdown, backup)
    Export {
        #[command(subcommand)]
        target: ExportTarget,
    },

    /// Show reading statistics
    Stats {
        /// Show stats for a specific year only
        #[arg(long)]
        year: Option<i32>,
    },
}

#[derive(Subcommand)]
enum ImportSource {
    /// Import from a Goodreads CSV export
    Goodreads {
        /// Path to the Goodreads CSV file
        path: PathBuf,

        /// Preview what would be imported without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Import from a Calibre library (metadata.db)
    Calibre {
        /// Path to the Calibre library directory (contains metadata.db)
        path: PathBuf,

        /// Preview what would be imported without writing
        #[arg(long)]
        dry_run: bool,

        /// Skip importing cover images
        #[arg(long)]
        no_covers: bool,
    },

    /// Undo a previous import by its ID
    Undo {
        /// Import ID (shown after a successful import)
        import_id: String,
    },
}

#[derive(Subcommand)]
enum ReadingAction {
    /// Start reading a book (WantToRead → Reading)
    Start {
        /// Book title
        book: String,
    },

    /// Finish reading a book (Reading → Read)
    Finish {
        /// Book title
        book: String,

        /// Rating (0–10, displayed as 5★ with half-star increments)
        #[arg(long, short)]
        rating: Option<i32>,
    },

    /// Abandon a book (Reading → Abandoned)
    Abandon {
        /// Book title
        book: String,
    },

    /// Put a book on hold (Reading → OnHold)
    Hold {
        /// Book title
        book: String,
    },

    /// Resume reading a book (OnHold/Abandoned → Reading)
    Resume {
        /// Book title
        book: String,
    },

    /// Log reading progress (page, percent, chapter, or duration)
    Update {
        /// Book title
        book: String,

        /// Page number reached
        #[arg(long)]
        page: Option<i32>,

        /// Percentage completed (0–100)
        #[arg(long)]
        percent: Option<i32>,

        /// Chapter number reached
        #[arg(long)]
        chapter: Option<i32>,

        /// Duration listened (e.g. 5h30m, 330m, 5.5h)
        #[arg(long)]
        duration: Option<String>,

        /// Optional note
        #[arg(long)]
        note: Option<String>,
    },

    /// Show reading log for a book
    Log {
        /// Book title
        book: String,
    },
}

#[derive(Subcommand)]
enum ShelfAction {
    /// Create a new shelf
    Create {
        /// Shelf name
        name: String,
    },

    /// Add books to a shelf
    Add {
        /// Shelf name
        shelf: String,

        /// Book titles to add
        #[arg(required = true)]
        books: Vec<String>,
    },

    /// Remove a book from a shelf
    Remove {
        /// Shelf name
        shelf: String,

        /// Book title to remove
        book: String,
    },

    /// List all shelves
    List,
}

#[derive(Subcommand)]
enum TagAction {
    /// Add a tag to books
    Add {
        /// Tag name
        tag: String,

        /// Book titles to tag
        #[arg(required = true)]
        books: Vec<String>,
    },

    /// Remove a tag from a book
    Remove {
        /// Tag name
        tag: String,

        /// Book title
        book: String,
    },

    /// List all tags with book counts
    List,
}

#[derive(Subcommand)]
enum ExportTarget {
    /// Export as CSV
    Csv {
        /// Output file path (stdout if omitted)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Export as JSON
    Json {
        /// Output file path (stdout if omitted)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Export as Markdown
    Markdown {
        /// Output file path (stdout if omitted)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Create a canonical backup (ZIP archive with library data and covers)
    Backup {
        /// Output file path (required)
        #[arg(long, short)]
        output: PathBuf,
    },
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Csv,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let data_dir = match &cli.data_dir {
        Some(dir) => dir.clone(),
        None => Database::default_data_dir().context("could not determine data directory")?,
    };

    // Resolve command (default: Browse = TUI)
    let command = cli.command.unwrap_or(Commands::Browse);

    // Commands that don't need the database
    match &command {
        Commands::Config { path, edit } => return cmd_config(&data_dir, *path, *edit),
        Commands::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), "toku", &mut std::io::stdout());
            return Ok(());
        }
        _ => {}
    }

    let db_path = data_dir.join("toku.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;
    let repo = BookRepository::new(&db);

    match command {
        Commands::Browse => tui::run(&repo),
        Commands::Add {
            title,
            author,
            isbn,
            book_format,
        } => cmd_add(
            &repo,
            &db_path,
            title,
            author,
            isbn,
            &book_format,
            &cli.format,
        ),
        Commands::Show { query } => cmd_show(&repo, &query, &cli.format),
        Commands::List { status, shelf, tag } => cmd_list(
            &repo,
            status.as_deref(),
            shelf.as_deref(),
            tag.as_deref(),
            &cli.format,
        ),
        Commands::Search {
            query,
            status,
            shelf,
            tag,
        } => cmd_search(
            &repo,
            &query,
            status.as_deref(),
            shelf.as_deref(),
            tag.as_deref(),
            &cli.format,
        ),
        Commands::Import { source } => cmd_import(&db, &repo, source, &cli.format),
        Commands::Reading { action } => cmd_reading(&repo, action, &cli.format),
        Commands::Shelf { action } => cmd_shelf(&repo, action, &cli.format),
        Commands::Tag { action } => cmd_tag(&repo, action, &cli.format),
        Commands::Export { target } => cmd_export(&db, &data_dir, target),
        Commands::Stats { year } => cmd_stats(&repo, year, &cli.format),
        // Already handled above
        Commands::Config { .. } | Commands::Completions { .. } => unreachable!(),
    }
}

fn cmd_config(data_dir: &Path, show_path: bool, open_edit: bool) -> Result<()> {
    let config_path = TokuConfig::config_path(data_dir);

    if show_path {
        println!("{}", config_path.display());
        return Ok(());
    }

    if open_edit {
        // Ensure the config file exists with defaults
        if !config_path.exists() {
            TokuConfig::default()
                .save(data_dir)
                .context("failed to create default config")?;
            eprintln!("Created default config at {}", config_path.display());
        }

        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| {
                if cfg!(windows) {
                    "notepad".to_string()
                } else {
                    "nano".to_string()
                }
            });

        std::process::Command::new(&editor)
            .arg(&config_path)
            .status()
            .with_context(|| format!("failed to open editor: {editor}"))?;
        return Ok(());
    }

    // Default: show current config
    let config = TokuConfig::load(data_dir).context("failed to load config")?;
    let toml_str = toml::to_string_pretty(&config).context("failed to serialize config")?;
    println!("{toml_str}");
    Ok(())
}

fn cmd_add(
    repo: &BookRepository,
    db_path: &Path,
    title: Option<String>,
    author: Option<String>,
    isbn: Option<String>,
    book_format: &str,
    output_format: &OutputFormat,
) -> Result<()> {
    let format: BookFormat = book_format
        .parse()
        .with_context(|| format!("invalid format: {book_format}"))?;

    if let Some(isbn_str) = &isbn {
        let validated = Isbn::parse(isbn_str).context("invalid ISBN")?;
        let isbn13 = validated.to_isbn13();

        // Check for existing book with this ISBN
        if let Some(existing) = repo.find_by_isbn(&isbn13)? {
            eprintln!("Book already exists: {} ({})", existing.title, existing.id);
            return Ok(());
        }

        // Fetch metadata from Open Library
        eprintln!("Fetching metadata for ISBN {isbn13}...");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let result = rt.block_on(toku_meta::fetch_by_isbn(&isbn13));

        match result {
            Ok(meta) => {
                let mut book = Book::new(&meta.title);
                book.subtitle = meta.subtitle;
                book.description = meta.description;
                book.page_count = meta.page_count;
                book.pub_date = meta.pub_date;
                book.language = meta.language;
                book.format = format;

                // Fetch cover
                let covers_dir = db_path.parent().unwrap().join("covers");
                if let Ok(Some(hash)) = rt.block_on(toku_meta::fetch_cover(&isbn13, &covers_dir)) {
                    book.cover_hash = Some(hash);
                }

                repo.create_book(&book)?;
                repo.add_isbn(&isbn13, &book.id)?;

                if let Some(isbn10) = validated.to_isbn10() {
                    repo.add_isbn(&isbn10, &book.id)?;
                }

                // Add authors
                for (i, name) in meta.authors.iter().enumerate() {
                    let a = Author::new(name.as_str());
                    repo.add_book_author(&a, &book.id, ContributorRole::Author, i as i32)?;
                }

                print_books(&[book], repo, output_format)?;
                eprintln!("✓ Added from Open Library");
            }
            Err(e) => {
                eprintln!("Could not fetch metadata: {e}");
                eprintln!("Adding with ISBN only. Use 'toku show' to verify.");
                let mut book = Book::new(isbn_str);
                book.format = format;
                repo.create_book(&book)?;
                repo.add_isbn(&isbn13, &book.id)?;
                print_books(&[book], repo, output_format)?;
            }
        }
    } else if let Some(title) = title {
        let mut book = Book::new(&title);
        book.format = format;
        repo.create_book(&book)?;

        if let Some(author_name) = author {
            let a = Author::new(author_name.as_str());
            repo.add_book_author(&a, &book.id, ContributorRole::Author, 0)?;
        }

        print_books(&[book], repo, output_format)?;
        eprintln!("✓ Added manually");
    } else {
        anyhow::bail!("provide --isbn or --title to add a book");
    }

    Ok(())
}

fn cmd_show(repo: &BookRepository, query: &str, output_format: &OutputFormat) -> Result<()> {
    let books = repo.search_books(query)?;
    if books.is_empty() {
        // Fall back to listing all and filtering by title contains
        let all = repo.list_books()?;
        let matched: Vec<Book> = all
            .into_iter()
            .filter(|b| b.title.to_lowercase().contains(&query.to_lowercase()))
            .collect();
        if matched.is_empty() {
            eprintln!("No book found matching \"{query}\"");
            return Ok(());
        }
        print_book_detail(&matched[0], repo, output_format)?;
    } else {
        print_book_detail(&books[0], repo, output_format)?;
    }
    Ok(())
}

fn cmd_list(
    repo: &BookRepository,
    status: Option<&str>,
    shelf: Option<&str>,
    tag: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    let books = if let Some(shelf_name) = shelf {
        repo.list_books_in_shelf(shelf_name)?
    } else if let Some(tag_name) = tag {
        repo.list_books_by_tag(tag_name)?
    } else {
        repo.list_books()?
    };

    let filtered: Vec<Book> = if let Some(s) = status {
        let target: toku_core::ReadingStatus = s.parse().context("invalid status")?;
        books.into_iter().filter(|b| b.status == target).collect()
    } else {
        books
    };

    if filtered.is_empty() {
        eprintln!("No books in your library yet. Add one with: toku add --isbn <isbn>");
        return Ok(());
    }

    print_books(&filtered, repo, output_format)?;
    eprintln!("\n{} book(s)", filtered.len());
    Ok(())
}

fn cmd_search(
    repo: &BookRepository,
    query: &str,
    status: Option<&str>,
    shelf: Option<&str>,
    tag: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    let books = repo.search_books_filtered(query, status, shelf, tag)?;
    if books.is_empty() {
        eprintln!("No results for \"{query}\"");
        return Ok(());
    }
    print_books(&books, repo, output_format)?;
    eprintln!("\n{} result(s)", books.len());
    Ok(())
}

// --- Output formatting ---

fn print_books(books: &[Book], repo: &BookRepository, output_format: &OutputFormat) -> Result<()> {
    match output_format {
        OutputFormat::Json => {
            #[derive(serde::Serialize)]
            struct BookOut {
                id: String,
                title: String,
                authors: Vec<String>,
                status: String,
                format: String,
                rating: Option<i32>,
                pages: Option<i32>,
            }
            let out: Vec<BookOut> = books
                .iter()
                .map(|b| {
                    let authors = repo
                        .get_book_authors(&b.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(a, _)| a.name)
                        .collect();
                    BookOut {
                        id: b.id.to_string(),
                        title: b.title.clone(),
                        authors,
                        status: b.status.as_str().to_string(),
                        format: b.format.as_str().to_string(),
                        rating: b.rating,
                        pages: b.page_count,
                    }
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        OutputFormat::Csv => {
            println!("title,authors,status,format,rating,pages");
            for b in books {
                let authors: Vec<String> = repo
                    .get_book_authors(&b.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(a, _)| a.name)
                    .collect();
                println!(
                    "\"{}\",\"{}\",{},{},{},{}",
                    b.title.replace('"', "\"\""),
                    authors.join("; ").replace('"', "\"\""),
                    b.status.as_str(),
                    b.format.as_str(),
                    b.rating.map_or("-".to_string(), |r| format!("{}", r)),
                    b.page_count.map_or("-".to_string(), |p| format!("{}", p)),
                );
            }
        }
        OutputFormat::Table => {
            use tabled::settings::Style;
            use tabled::{Table, Tabled};

            // Detect terminal width (fallback: 120 columns)
            let term_width = crossterm::terminal::size()
                .map(|(w, _)| w as usize)
                .unwrap_or(120);

            // Width budget: 7 borders + 12 padding = 19 overhead
            // Fixed columns: Status(7) + Fmt(5) + ★(3) + Pages(5) = 20
            // Remaining goes to Title (60%) and Author (40%)
            let fixed_overhead = 39;
            let flexible = term_width.saturating_sub(fixed_overhead);
            let title_max = (flexible * 3 / 5).max(12);
            let author_max = flexible.saturating_sub(title_max).max(8);

            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Title")]
                title: String,
                #[tabled(rename = "Author")]
                author: String,
                #[tabled(rename = "Status")]
                status: String,
                #[tabled(rename = "Fmt")]
                format: String,
                #[tabled(rename = "★")]
                rating: String,
                #[tabled(rename = "Pages")]
                pages: String,
            }

            let rows: Vec<Row> = books
                .iter()
                .map(|b| {
                    let authors: Vec<String> = repo
                        .get_book_authors(&b.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(a, _)| a.name)
                        .collect();

                    let status = match b.status {
                        ReadingStatus::WantToRead => "Want",
                        ReadingStatus::Reading => "Reading",
                        ReadingStatus::Read => "Read",
                        ReadingStatus::Abandoned => "DNF",
                        ReadingStatus::OnHold => "On Hold",
                    };

                    let format = match b.format {
                        BookFormat::Physical => "Print",
                        BookFormat::Ebook => "Ebook",
                        BookFormat::Audiobook => "Audio",
                    };

                    Row {
                        title: import_ui::truncate_str(&b.title, title_max),
                        author: import_ui::truncate_str(&authors.join(", "), author_max),
                        status: status.to_string(),
                        format: format.to_string(),
                        rating: b
                            .rating
                            .map_or("—".to_string(), |r| format!("{:.1}", r as f32 / 2.0)),
                        pages: b.page_count.map_or("—".to_string(), |p| p.to_string()),
                    }
                })
                .collect();

            let mut table = Table::new(rows);
            table.with(Style::rounded());
            println!("{table}");
        }
    }
    Ok(())
}

fn print_book_detail(
    book: &Book,
    repo: &BookRepository,
    output_format: &OutputFormat,
) -> Result<()> {
    match output_format {
        OutputFormat::Json => {
            print_books(std::slice::from_ref(book), repo, output_format)?;
        }
        OutputFormat::Csv => {
            print_books(std::slice::from_ref(book), repo, output_format)?;
        }
        OutputFormat::Table => {
            let authors: Vec<String> = repo
                .get_book_authors(&book.id)
                .unwrap_or_default()
                .into_iter()
                .map(|(a, ba)| {
                    if ba.role == ContributorRole::Author {
                        a.name
                    } else {
                        format!("{} ({})", a.name, ba.role)
                    }
                })
                .collect();

            println!("  Title:   {}", book.title);
            if let Some(sub) = &book.subtitle {
                println!("  Sub:     {sub}");
            }
            println!(
                "  Author:  {}",
                if authors.is_empty() {
                    "—".to_string()
                } else {
                    authors.join(", ")
                }
            );
            println!("  Status:  {}", book.status);
            println!("  Format:  {}", book.format);
            if let Some(pages) = book.page_count {
                println!("  Pages:   {pages}");
            }
            if let Some(dur) = book.duration_minutes {
                println!("  Length:  {}h {}m", dur / 60, dur % 60);
            }
            if let Some(rating) = book.rating {
                println!("  Rating:  {:.1}★", rating as f32 / 2.0);
            }
            if let Some(date) = &book.pub_date {
                println!("  Pub:     {date}");
            }
            if let Some(lang) = &book.language {
                println!("  Lang:    {lang}");
            }
            if let Some(hash) = &book.cover_hash {
                println!("  Cover:   ✓ ({hash})");
            }
            if let Some(desc) = &book.description {
                let truncated = {
                    let char_count = desc.chars().count();
                    if char_count > 200 {
                        let t: String = desc.chars().take(199).collect();
                        format!("{t}…")
                    } else {
                        desc.clone()
                    }
                };
                println!("  Desc:    {truncated}");
            }
            println!("  ID:      {}", book.id);
        }
    }
    Ok(())
}

fn cmd_import(
    db: &Database,
    _repo: &BookRepository,
    source: ImportSource,
    output_format: &OutputFormat,
) -> Result<()> {
    match source {
        ImportSource::Goodreads { path, dry_run } => {
            import_ui::run_goodreads_import(db, &path, dry_run, output_format)
        }
        ImportSource::Calibre {
            path,
            dry_run,
            no_covers,
        } => {
            if !path.exists() {
                anyhow::bail!("directory not found: {}", path.display());
            }

            let opts = toku_import::CalibreImportOptions {
                dry_run,
                import_covers: !no_covers,
            };

            if dry_run {
                eprintln!("Dry run — no changes will be made:\n");
            } else {
                eprintln!("Importing from Calibre library: {}\n", path.display());
            }

            let report =
                toku_import::import_calibre(db, &path, &opts).context("Calibre import failed")?;

            eprintln!("\n{report}");

            if let Some(ref id) = report.import_id {
                eprintln!("Import ID: {id}");
                eprintln!("To undo: toku import undo {id}");
            }

            Ok(())
        }
        ImportSource::Undo { import_id } => {
            let count =
                toku_import::undo_import(db, &import_id).context("failed to undo import")?;

            if count > 0 {
                eprintln!("✓ Undone: removed {count} book(s) from import {import_id}");
            } else {
                eprintln!("No books found for import ID {import_id}");
            }

            Ok(())
        }
    }
}

fn cmd_export(db: &Database, data_dir: &Path, target: ExportTarget) -> Result<()> {
    match target {
        ExportTarget::Csv { output } => {
            if let Some(path) = output {
                let file = std::fs::File::create(&path)
                    .with_context(|| format!("failed to create {}", path.display()))?;
                toku_export::export_csv(db, file).context("CSV export failed")?;
                eprintln!("✓ Exported CSV to {}", path.display());
            } else {
                toku_export::export_csv(db, std::io::stdout()).context("CSV export failed")?;
            }
            Ok(())
        }
        ExportTarget::Json { output } => {
            if let Some(path) = output {
                let file = std::fs::File::create(&path)
                    .with_context(|| format!("failed to create {}", path.display()))?;
                toku_export::export_json(db, file).context("JSON export failed")?;
                eprintln!("✓ Exported JSON to {}", path.display());
            } else {
                toku_export::export_json(db, std::io::stdout()).context("JSON export failed")?;
            }
            Ok(())
        }
        ExportTarget::Markdown { output } => {
            if let Some(path) = output {
                let file = std::fs::File::create(&path)
                    .with_context(|| format!("failed to create {}", path.display()))?;
                toku_export::export_markdown(db, file).context("Markdown export failed")?;
                eprintln!("✓ Exported Markdown to {}", path.display());
            } else {
                toku_export::export_markdown(db, std::io::stdout())
                    .context("Markdown export failed")?;
            }
            Ok(())
        }
        ExportTarget::Backup { output } => {
            toku_export::export_backup(db, data_dir, &output).context("backup export failed")?;
            eprintln!("✓ Backup saved to {}", output.display());
            Ok(())
        }
    }
}

/// Resolve a book title to a Book, returning a user-friendly error if not found.
fn resolve_book(repo: &BookRepository, title: &str) -> Result<Book> {
    if let Some(book) = repo.find_book_by_title(title)? {
        return Ok(book);
    }

    // Fall back to substring search
    let all = repo.list_books()?;
    let matches: Vec<&Book> = all
        .iter()
        .filter(|b| b.title.to_lowercase().contains(&title.to_lowercase()))
        .collect();

    match matches.len() {
        0 => anyhow::bail!("no book found matching \"{title}\""),
        1 => Ok(matches[0].clone()),
        _ => {
            eprintln!("Multiple books match \"{title}\":");
            for b in &matches {
                eprintln!("  • {} ({})", b.title, b.status);
            }
            anyhow::bail!("be more specific or use the exact title")
        }
    }
}

fn cmd_reading(
    repo: &BookRepository,
    action: ReadingAction,
    output_format: &OutputFormat,
) -> Result<()> {
    match action {
        ReadingAction::Start { book: title } => {
            let book = resolve_book(repo, &title)?;
            let target = ReadingStatus::Reading;

            if !book.status.can_transition_to(&target) {
                anyhow::bail!(
                    "cannot start: \"{}\" is currently {} (expected want-to-read, on-hold, abandoned, or read)",
                    book.title,
                    book.status
                );
            }

            repo.update_book_status(&book.id, target)?;

            let session = ReadingSession::new(book.id);
            repo.create_reading_session(&session)?;

            eprintln!("✓ Started reading \"{}\"", book.title);
            Ok(())
        }

        ReadingAction::Finish {
            book: title,
            rating,
        } => {
            let book = resolve_book(repo, &title)?;
            let target = ReadingStatus::Read;

            if !book.status.can_transition_to(&target) {
                anyhow::bail!(
                    "cannot finish: \"{}\" is currently {} (expected reading)",
                    book.title,
                    book.status
                );
            }

            if let Some(r) = rating {
                if !(0..=10).contains(&r) {
                    anyhow::bail!("rating must be between 0 and 10, got {r}");
                }
                repo.update_book_rating(&book.id, r)?;
            }

            repo.update_book_status(&book.id, target)?;

            match rating {
                Some(r) => eprintln!(
                    "✓ Finished \"{}\" — rated {:.1}★",
                    book.title,
                    r as f32 / 2.0
                ),
                None => eprintln!("✓ Finished \"{}\"", book.title),
            }
            Ok(())
        }

        ReadingAction::Abandon { book: title } => {
            let book = resolve_book(repo, &title)?;
            let target = ReadingStatus::Abandoned;

            if !book.status.can_transition_to(&target) {
                anyhow::bail!(
                    "cannot abandon: \"{}\" is currently {} (expected reading)",
                    book.title,
                    book.status
                );
            }

            repo.update_book_status(&book.id, target)?;
            eprintln!("✓ Abandoned \"{}\"", book.title);
            Ok(())
        }

        ReadingAction::Hold { book: title } => {
            let book = resolve_book(repo, &title)?;
            let target = ReadingStatus::OnHold;

            if !book.status.can_transition_to(&target) {
                anyhow::bail!(
                    "cannot hold: \"{}\" is currently {} (expected reading)",
                    book.title,
                    book.status
                );
            }

            repo.update_book_status(&book.id, target)?;
            eprintln!("✓ Put \"{}\" on hold", book.title);
            Ok(())
        }

        ReadingAction::Resume { book: title } => {
            let book = resolve_book(repo, &title)?;
            let target = ReadingStatus::Reading;

            if !book.status.can_transition_to(&target) {
                anyhow::bail!(
                    "cannot resume: \"{}\" is currently {} (expected on-hold or abandoned)",
                    book.title,
                    book.status
                );
            }

            repo.update_book_status(&book.id, target)?;

            let session = ReadingSession::new(book.id);
            repo.create_reading_session(&session)?;

            eprintln!("✓ Resumed reading \"{}\"", book.title);
            Ok(())
        }

        ReadingAction::Update {
            book: title,
            page,
            percent,
            chapter,
            duration,
            note,
        } => {
            let book = resolve_book(repo, &title)?;

            if book.status != ReadingStatus::Reading {
                anyhow::bail!(
                    "cannot update progress: \"{}\" is currently {} (expected reading)",
                    book.title,
                    book.status
                );
            }

            let (progress_type, value) = if let Some(p) = page {
                (ProgressType::Page, p)
            } else if let Some(pct) = percent {
                if !(0..=100).contains(&pct) {
                    anyhow::bail!("percentage must be between 0 and 100, got {pct}");
                }
                (ProgressType::Percent, pct)
            } else if let Some(ch) = chapter {
                (ProgressType::Chapter, ch)
            } else if let Some(dur_str) = duration {
                let minutes = parse_duration_to_minutes(&dur_str)
                    .with_context(|| format!("invalid duration: {dur_str}"))?;
                (ProgressType::Duration, minutes)
            } else {
                anyhow::bail!("specify one of --page, --percent, --chapter, or --duration");
            };

            let active_session = repo.get_active_session(&book.id)?;

            let mut progress = ReadingProgress::new(book.id, progress_type, value);
            progress.session_id = active_session.map(|s| s.id);
            progress.note = note;
            repo.log_progress(&progress)?;

            let label = match progress_type {
                ProgressType::Page => format!("Page {value}"),
                ProgressType::Percent => format!("{value}%"),
                ProgressType::Chapter => format!("Chapter {value}"),
                ProgressType::Duration => {
                    let h = value / 60;
                    let m = value % 60;
                    if h > 0 {
                        format!("{h}h {m}m")
                    } else {
                        format!("{m}m")
                    }
                }
            };

            eprintln!("✓ {label} of \"{}\" logged", book.title);
            Ok(())
        }

        ReadingAction::Log { book: title } => {
            let book = resolve_book(repo, &title)?;
            let log = repo.get_reading_log(&book.id)?;

            if log.is_empty() {
                eprintln!("No reading progress logged for \"{}\"", book.title);
                return Ok(());
            }

            match output_format {
                OutputFormat::Json => {
                    #[derive(serde::Serialize)]
                    struct ProgressOut {
                        date: String,
                        progress_type: String,
                        value: i32,
                        note: Option<String>,
                    }
                    let out: Vec<ProgressOut> = log
                        .iter()
                        .map(|p| ProgressOut {
                            date: p.logged_at.format("%Y-%m-%d %H:%M").to_string(),
                            progress_type: p.progress_type.as_str().to_string(),
                            value: p.value,
                            note: p.note.clone(),
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Csv => {
                    println!("date,type,value,note");
                    for p in &log {
                        println!(
                            "{},{},{},\"{}\"",
                            p.logged_at.format("%Y-%m-%d %H:%M"),
                            p.progress_type.as_str(),
                            p.value,
                            p.note.as_deref().unwrap_or("").replace('"', "\"\""),
                        );
                    }
                }
                OutputFormat::Table => {
                    use tabled::{Table, Tabled};

                    #[derive(Tabled)]
                    struct Row {
                        #[tabled(rename = "Date")]
                        date: String,
                        #[tabled(rename = "Type")]
                        progress_type: String,
                        #[tabled(rename = "Value")]
                        value: String,
                        #[tabled(rename = "Note")]
                        note: String,
                    }

                    let rows: Vec<Row> = log
                        .iter()
                        .map(|p| {
                            let value_str = match p.progress_type {
                                ProgressType::Page => format!("p. {}", p.value),
                                ProgressType::Percent => format!("{}%", p.value),
                                ProgressType::Chapter => format!("ch. {}", p.value),
                                ProgressType::Duration => {
                                    let h = p.value / 60;
                                    let m = p.value % 60;
                                    if h > 0 {
                                        format!("{h}h {m}m")
                                    } else {
                                        format!("{m}m")
                                    }
                                }
                            };

                            Row {
                                date: p.logged_at.format("%Y-%m-%d %H:%M").to_string(),
                                progress_type: p.progress_type.as_str().to_string(),
                                value: value_str,
                                note: p.note.clone().unwrap_or_default(),
                            }
                        })
                        .collect();

                    println!("{}", Table::new(rows));
                }
            }

            eprintln!("\n{} entries for \"{}\"", log.len(), book.title);
            Ok(())
        }
    }
}

fn cmd_shelf(
    repo: &BookRepository,
    action: ShelfAction,
    output_format: &OutputFormat,
) -> Result<()> {
    match action {
        ShelfAction::Create { name } => {
            repo.create_shelf(&name)?;
            eprintln!("✓ Created shelf \"{name}\"");
            Ok(())
        }
        ShelfAction::Add { shelf, books } => {
            for title in &books {
                let book = resolve_book(repo, title)?;
                repo.add_book_to_shelf(&book.id, &shelf)?;
                eprintln!("✓ Added \"{}\" to shelf \"{shelf}\"", book.title);
            }
            Ok(())
        }
        ShelfAction::Remove { shelf, book: title } => {
            let book = resolve_book(repo, &title)?;
            repo.remove_book_from_shelf(&book.id, &shelf)?;
            eprintln!("✓ Removed \"{}\" from shelf \"{shelf}\"", book.title);
            Ok(())
        }
        ShelfAction::List => {
            let shelves = repo.list_shelves()?;
            if shelves.is_empty() {
                eprintln!("No shelves yet. Create one with: toku shelf create <name>");
                return Ok(());
            }

            match output_format {
                OutputFormat::Json => {
                    #[derive(serde::Serialize)]
                    struct ShelfOut {
                        name: String,
                        books: usize,
                    }
                    let out: Vec<ShelfOut> = shelves
                        .iter()
                        .map(|s| {
                            let count = repo
                                .list_books_in_shelf(&s.name)
                                .map(|b| b.len())
                                .unwrap_or(0);
                            ShelfOut {
                                name: s.name.clone(),
                                books: count,
                            }
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Csv => {
                    println!("name,books");
                    for s in &shelves {
                        let count = repo
                            .list_books_in_shelf(&s.name)
                            .map(|b| b.len())
                            .unwrap_or(0);
                        println!("\"{}\",{}", s.name.replace('"', "\"\""), count);
                    }
                }
                OutputFormat::Table => {
                    use tabled::{Table, Tabled};

                    #[derive(Tabled)]
                    struct Row {
                        #[tabled(rename = "Shelf")]
                        name: String,
                        #[tabled(rename = "Books")]
                        books: usize,
                    }

                    let rows: Vec<Row> = shelves
                        .iter()
                        .map(|s| {
                            let count = repo
                                .list_books_in_shelf(&s.name)
                                .map(|b| b.len())
                                .unwrap_or(0);
                            Row {
                                name: s.name.clone(),
                                books: count,
                            }
                        })
                        .collect();

                    println!("{}", Table::new(rows));
                }
            }
            eprintln!("\n{} shelf(s)", shelves.len());
            Ok(())
        }
    }
}

fn cmd_tag(repo: &BookRepository, action: TagAction, output_format: &OutputFormat) -> Result<()> {
    match action {
        TagAction::Add { tag, books } => {
            for title in &books {
                let book = resolve_book(repo, title)?;
                repo.add_tag_to_book(&book.id, &tag)?;
                eprintln!("✓ Tagged \"{}\" with \"{tag}\"", book.title);
            }
            Ok(())
        }
        TagAction::Remove { tag, book: title } => {
            let book = resolve_book(repo, &title)?;
            repo.remove_tag_from_book(&book.id, &tag)?;
            eprintln!("✓ Removed tag \"{tag}\" from \"{}\"", book.title);
            Ok(())
        }
        TagAction::List => {
            let tags = repo.list_tags_with_counts()?;
            if tags.is_empty() {
                eprintln!("No tags yet. Add one with: toku tag add <tag> <book>");
                return Ok(());
            }

            match output_format {
                OutputFormat::Json => {
                    #[derive(serde::Serialize)]
                    struct TagOut {
                        name: String,
                        books: i64,
                    }
                    let out: Vec<TagOut> = tags
                        .iter()
                        .map(|(t, count)| TagOut {
                            name: t.name.clone(),
                            books: *count,
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Csv => {
                    println!("tag,books");
                    for (t, count) in &tags {
                        println!("\"{}\",{}", t.name.replace('"', "\"\""), count);
                    }
                }
                OutputFormat::Table => {
                    use tabled::{Table, Tabled};

                    #[derive(Tabled)]
                    struct Row {
                        #[tabled(rename = "Tag")]
                        name: String,
                        #[tabled(rename = "Books")]
                        books: i64,
                    }

                    let rows: Vec<Row> = tags
                        .iter()
                        .map(|(t, count)| Row {
                            name: t.name.clone(),
                            books: *count,
                        })
                        .collect();

                    println!("{}", Table::new(rows));
                }
            }
            eprintln!("\n{} tag(s)", tags.len());
            Ok(())
        }
    }
}

fn cmd_stats(repo: &BookRepository, year: Option<i32>, output_format: &OutputFormat) -> Result<()> {
    let books = repo.list_books()?;

    let sessions = match year {
        Some(y) => repo.list_reading_sessions_in_year(y)?,
        None => repo.list_reading_sessions()?,
    };

    let currently_reading_details = repo.get_currently_reading_details()?;
    let currently_reading_input: Vec<CurrentlyReadingInput> = currently_reading_details
        .into_iter()
        .map(|(book, progress, authors)| {
            let author = authors
                .into_iter()
                .map(|(a, _)| a.name)
                .collect::<Vec<_>>()
                .join(", ");
            CurrentlyReadingInput {
                title: book.title,
                author,
                page_count: book.page_count,
                latest_progress: progress,
            }
        })
        .collect();

    let now = chrono::Utc::now();
    let stats = compute_stats(&books, &sessions, &currently_reading_input, now);

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        OutputFormat::Csv => {
            println!("metric,value");
            println!("total_books,{}", stats.total_books);
            println!("books_read,{}", stats.books_read);
            println!("books_reading,{}", stats.books_reading);
            println!("books_want_to_read,{}", stats.books_want_to_read);
            println!("books_abandoned,{}", stats.books_abandoned);
            println!("total_pages_read,{}", stats.total_pages_read);
            println!(
                "average_rating,{}",
                stats
                    .average_rating
                    .map_or("-".to_string(), |r| format!("{r:.1}"))
            );
            println!(
                "average_rating_stars,{}",
                stats
                    .average_rating_stars
                    .map_or("-".to_string(), |r| format!("{r:.1}"))
            );
            println!("books_per_month,{:.1}", stats.books_per_month);
            println!("pages_per_day,{:.1}", stats.pages_per_day);
            println!("format_physical,{}", stats.format_breakdown.physical);
            println!("format_ebook,{}", stats.format_breakdown.ebook);
            println!("format_audiobook,{}", stats.format_breakdown.audiobook);
        }
        OutputFormat::Table => {
            let header = match year {
                Some(y) => format!("📊 Reading Statistics ({y})"),
                None => "📊 Reading Statistics (All Time)".to_string(),
            };
            println!("\n{header}\n");

            println!("  Books read:        {:>5}", stats.books_read);
            println!("  Currently reading: {:>5}", stats.books_reading);
            println!("  Want to read:      {:>5}", stats.books_want_to_read);
            println!("  Abandoned:         {:>5}", stats.books_abandoned);
            println!();

            println!(
                "  Pages read:        {:>5}",
                format_number(stats.total_pages_read)
            );
            println!(
                "  Average rating:    {:>5}",
                stats
                    .average_rating_stars
                    .map_or("—".to_string(), |r| format!("{r:.1}★"))
            );
            println!(
                "  Reading pace:      {:>5} books/month",
                format!("{:.1}", stats.books_per_month)
            );
            println!(
                "                     {:>5} pages/day",
                format!("{:.1}", stats.pages_per_day)
            );

            let total_formats = stats.format_breakdown.physical
                + stats.format_breakdown.ebook
                + stats.format_breakdown.audiobook;

            if total_formats > 0 {
                let pct_physical = (stats.format_breakdown.physical * 100)
                    .checked_div(total_formats)
                    .unwrap_or(0);
                let pct_ebook = (stats.format_breakdown.ebook * 100)
                    .checked_div(total_formats)
                    .unwrap_or(0);
                let pct_audiobook = (stats.format_breakdown.audiobook * 100)
                    .checked_div(total_formats)
                    .unwrap_or(0);

                println!();
                println!("  Format breakdown:");
                println!(
                    "    Physical:  {:>3} ({pct_physical}%)",
                    stats.format_breakdown.physical,
                );
                println!(
                    "    Ebook:     {:>3} ({pct_ebook}%)",
                    stats.format_breakdown.ebook,
                );
                println!(
                    "    Audiobook: {:>3} ({pct_audiobook}%)",
                    stats.format_breakdown.audiobook,
                );
            }

            if !stats.currently_reading.is_empty() {
                println!();
                println!("  Currently reading:");
                for cr in &stats.currently_reading {
                    let progress = match (cr.latest_page, cr.total_pages, cr.percent) {
                        (Some(page), Some(total), Some(pct)) => {
                            format!(" (page {page}/{total}, {pct:.0}%)")
                        }
                        (Some(page), None, _) => format!(" (page {page})"),
                        _ => String::new(),
                    };
                    let author = if cr.author.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", cr.author)
                    };
                    println!("    {}{author}{progress}", cr.title);
                }
            }

            println!();
        }
    }

    Ok(())
}

fn format_number(n: i64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
