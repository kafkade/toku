use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use toku_core::{Author, Book, BookFormat, ContributorRole, Isbn};
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
    },

    /// Search your library
    Search {
        /// Search query
        query: String,
    },

    /// Import books from external sources
    Import {
        #[command(subcommand)]
        source: ImportSource,
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

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Csv,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let db_path = match &cli.data_dir {
        Some(dir) => dir.join("toku.db"),
        None => Database::default_db_path().context("could not determine data directory")?,
    };

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
        Commands::List { status } => cmd_list(&repo, status.as_deref(), &cli.format),
        Commands::Search { query } => cmd_search(&repo, &query, &cli.format),
        Commands::Import { source } => cmd_import(&db, &repo, source, &cli.format),
    }
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
    output_format: &OutputFormat,
) -> Result<()> {
    let books = repo.list_books()?;
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

fn cmd_search(repo: &BookRepository, query: &str, output_format: &OutputFormat) -> Result<()> {
    let books = repo.search_books(query)?;
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
