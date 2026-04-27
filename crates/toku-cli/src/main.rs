use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use toku_core::{
    Author, Book, BookFormat, ContributorRole, Isbn, ReadingSession, ReadingStatus, TokuConfig,
};
use toku_db::{BookRepository, Database};

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
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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

    // Commands that don't need the database
    match &cli.command {
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

    match cli.command {
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
        Commands::Reading { action } => cmd_reading(&repo, action),
        Commands::Shelf { action } => cmd_shelf(&repo, action, &cli.format),
        Commands::Tag { action } => cmd_tag(&repo, action, &cli.format),
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
            use tabled::{Table, Tabled};

            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Title")]
                title: String,
                #[tabled(rename = "Author")]
                author: String,
                #[tabled(rename = "Status")]
                status: String,
                #[tabled(rename = "Format")]
                format: String,
                #[tabled(rename = "Rating")]
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

                    let title = if b.title.len() > 40 {
                        format!("{}…", &b.title[..39])
                    } else {
                        b.title.clone()
                    };

                    Row {
                        title,
                        author: authors.join(", "),
                        status: b.status.as_str().to_string(),
                        format: b.format.as_str().to_string(),
                        rating: b
                            .rating
                            .map_or("—".to_string(), |r| format!("{:.1}★", r as f32 / 2.0)),
                        pages: b.page_count.map_or("—".to_string(), |p| p.to_string()),
                    }
                })
                .collect();

            println!("{}", Table::new(rows));
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
                let truncated = if desc.len() > 200 {
                    format!("{}…", &desc[..200])
                } else {
                    desc.clone()
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
    repo: &BookRepository,
    source: ImportSource,
    output_format: &OutputFormat,
) -> Result<()> {
    match source {
        ImportSource::Goodreads { path, dry_run } => {
            if !path.exists() {
                anyhow::bail!("file not found: {}", path.display());
            }

            let opts = toku_import::GoodreadsImportOptions { dry_run };

            if dry_run {
                eprintln!("Dry run — no changes will be made:\n");
            } else {
                eprintln!("Importing from Goodreads CSV: {}\n", path.display());
            }

            let report = toku_import::import_goodreads(db, &path, &opts)
                .context("Goodreads import failed")?;

            eprintln!("\n{report}");

            if let Some(ref id) = report.import_id {
                eprintln!("Import ID: {id}");
                eprintln!("To undo: toku import undo {id}");
            }

            if !dry_run && report.imported > 0 {
                eprintln!();
                let books = repo.list_books()?;
                print_books(&books, repo, output_format)?;
                eprintln!("\n{} book(s) in library", books.len());
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

fn cmd_reading(repo: &BookRepository, action: ReadingAction) -> Result<()> {
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
