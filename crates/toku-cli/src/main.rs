use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use base64::Engine as _;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use toku_core::{
    Author, Book, BookFormat, ContributorRole, CurrentlyReadingInput, Isbn, PaceRating,
    ProgressType, ReadingProgress, ReadingSession, ReadingStatus, SmartFilter, StatsInput, TagType,
    TokuConfig, compute_stats, parse_duration_to_minutes,
};
use toku_db::{BookRepository, Database};
use toku_files::{
    Converter, EbookFile, FileFormat, FileRepository, UsageTotals, VerifyStatus, sha256_file,
    usage_by_key, usage_totals, verify_file,
};
mod account;
mod import_ui;
mod sync;
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

        /// Tag(s) to apply (repeatable)
        #[arg(long, short = 'T')]
        tag: Vec<String>,

        /// Mood tag(s) to apply (repeatable)
        #[arg(long)]
        mood: Vec<String>,

        /// Pace rating (fast, medium, slow)
        #[arg(long)]
        pace: Option<String>,

        /// Content warning(s) to apply (repeatable)
        #[arg(long, alias = "cw")]
        content_warning: Vec<String>,

        /// Initial reading status (want-to-read, reading, read, abandoned, on-hold)
        #[arg(long)]
        status: Option<String>,
    },

    /// Show details of a book
    Show {
        /// Book title or ID to display
        query: String,
    },

    /// Edit book metadata (mood tags, pace, content warnings, rating)
    Edit {
        /// Book title or ID
        query: String,

        /// Mood tag(s) to add (repeatable)
        #[arg(long)]
        mood: Vec<String>,

        /// Pace rating (fast, medium, slow)
        #[arg(long)]
        pace: Option<String>,

        /// Content warning(s) to add (repeatable)
        #[arg(long, alias = "cw")]
        content_warning: Vec<String>,

        /// Mood tag(s) to remove (repeatable)
        #[arg(long)]
        remove_mood: Vec<String>,

        /// Content warning(s) to remove (repeatable)
        #[arg(long, alias = "remove-cw")]
        remove_content_warning: Vec<String>,

        /// Set rating (0–10, displayed as 5★ with half-star increments)
        #[arg(long, short)]
        rating: Option<i32>,
    },

    /// List books in your library
    List {
        /// Filter by reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by tag name
        #[arg(long)]
        tag: Option<String>,

        /// Filter by mood tag(s) (same-type OR, cross-type AND)
        #[arg(long)]
        mood: Vec<String>,

        /// Filter by pace (fast, medium, slow)
        #[arg(long)]
        pace: Option<String>,
    },

    /// Search your library
    Search {
        /// Search query
        query: String,

        /// Filter by reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by tag name
        #[arg(long)]
        tag: Option<String>,
    },

    /// Import books from external sources
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },

    /// Search Open Library for books online
    Lookup {
        /// Search query (title, author, or keywords)
        query: String,

        /// Maximum number of results to show
        #[arg(long, default_value = "10")]
        limit: usize,
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

    /// Manage tags for categorizing books
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },

    /// Associate ebook files (.epub/.pdf/.mobi/.azw3) with books
    File {
        #[command(subcommand)]
        action: FileAction,
    },

    /// Convert an associated ebook to another format (optional; needs Calibre)
    ///
    /// Shells out to Calibre's `ebook-convert`. The resulting file is written
    /// next to the source file and auto-associated with the book. Requires
    /// Calibre on your PATH (https://calibre-ebook.com/download); it is never a
    /// hard dependency. DRM-free files only — no DRM stripping.
    Convert {
        /// Book title (or id) whose file should be converted
        book: String,

        /// Target format to convert to (epub/pdf/mobi/azw3)
        #[arg(long)]
        to: String,

        /// Source format to convert from (auto-detected when the book has one file)
        #[arg(long)]
        from: Option<String>,

        /// Overwrite the output file if it already exists on disk
        #[arg(long)]
        force: bool,
    },

    /// Export your library (csv, json, markdown, backup)
    Export {
        #[command(subcommand)]
        target: ExportTarget,
    },

    /// Bulk operations on multiple books
    Bulk {
        #[command(subcommand)]
        action: BulkAction,
    },

    /// Show reading statistics
    Stats {
        /// Show stats for a specific year only
        #[arg(long)]
        year: Option<i32>,

        /// Filter stats to a specific author (case-insensitive)
        #[arg(long)]
        author: Option<String>,

        /// Show mood tag distribution over time
        #[arg(long)]
        mood_trends: bool,
    },

    /// Start the web dashboard server
    Serve {
        /// Port to listen on
        #[arg(long, short, default_value = "3000")]
        port: u16,

        /// Host to bind (use 127.0.0.1 for local-only access)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Run in hosted mode: require account login + sessions (needed for
        /// any non-loopback / network-facing deployment).
        #[arg(long, env = "TOKU_WEB_HOSTED")]
        hosted: bool,

        /// Mark auth cookies `Secure` (default in hosted mode). Disable only
        /// for plain-HTTP testing of hosted mode on localhost.
        #[arg(long, env = "TOKU_WEB_INSECURE_COOKIES")]
        insecure_cookies: bool,
    },

    /// Serve the library as an OPDS catalog for e-readers (KOReader, etc.)
    ///
    /// Exposes associated ebook files over OPDS so e-reader apps can browse and
    /// download them. Local-first and LAN-facing: it makes no external calls and
    /// serves only your local library. Binds all interfaces by default so
    /// e-readers on your network can reach it; guard it with optional HTTP Basic
    /// auth via `toku opds set-password`.
    Opds {
        #[command(subcommand)]
        action: OpdsAction,
    },

    /// Manage work grouping (link editions of the same creative work)
    Work {
        #[command(subcommand)]
        action: WorkAction,
    },

    /// Merge duplicate books (keep one, move all data from the other)
    Merge {
        /// Book to keep (title or ID)
        keep: String,

        /// Book to remove (title or ID) — its data is moved to the kept book
        remove: String,
    },

    /// Manage shelves (regular and smart)
    Shelf {
        #[command(subcommand)]
        action: ShelfAction,
    },

    /// Manage sync with a toku-sync server
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },

    /// Manage account secrets (Secret Key, Emergency Kit)
    Account {
        #[command(subcommand)]
        action: AccountAction,
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

    /// Import from a StoryGraph CSV export
    Storygraph {
        /// Path to the StoryGraph CSV file
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

    /// Restore from a canonical lossless backup (`.zip`)
    Backup {
        /// Path to the backup ZIP created by `toku export backup`
        path: PathBuf,

        /// Replace the current library verbatim (disaster recovery) instead of
        /// the default additive, precedence-respecting merge
        #[arg(long)]
        replace: bool,

        /// Preview what would be restored without writing
        #[arg(long)]
        dry_run: bool,
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
enum OpdsAction {
    /// Start the OPDS catalog server
    Serve {
        /// Port to listen on
        #[arg(long, short, default_value = "3001")]
        port: u16,

        /// Host to bind. Defaults to 0.0.0.0 so e-readers on your local network
        /// can reach the catalog; use 127.0.0.1 to restrict to this machine.
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
    },

    /// Enable HTTP Basic auth by setting a username + password (prompts for the
    /// password, stores a salted hash in the `[opds]` config section)
    SetPassword {
        /// Username for HTTP Basic auth
        username: String,
    },

    /// Disable HTTP Basic auth (clears the `[opds]` credentials)
    Disable,
}

#[derive(Subcommand)]
enum FileAction {
    /// Associate an ebook file with a book (format auto-detected from extension)
    Add {
        /// Book title
        book: String,

        /// Path to the ebook file
        path: PathBuf,
    },

    /// List files associated with a book
    List {
        /// Book title
        book: String,
    },

    /// Remove a file association by format or path
    Remove {
        /// Book title
        book: String,

        /// File format (epub/pdf/mobi/azw3) or exact path to remove
        target: String,

        /// Also delete the file from disk (default: only drop the DB record)
        #[arg(long)]
        delete_file: bool,
    },

    /// Organize associated files on disk using the configured path template
    Organize {
        /// Book title to organize (omit and use --all to organize everything)
        book: Option<String>,

        /// Organize files for every book in the library
        #[arg(long)]
        all: bool,

        /// Preview the planned moves without touching disk or the database
        #[arg(long)]
        dry_run: bool,

        /// Copy files into the library instead of moving them
        #[arg(long)]
        copy: bool,
    },

    /// Verify file integrity by recomputing SHA-256 checksums
    ///
    /// Streams each associated file to recompute its checksum and compares it to
    /// the value stored when the file was linked. Flags files whose contents
    /// changed (mismatch) or that are no longer on disk (missing). Exits with a
    /// non-zero status if any problem is found, for use in scripts.
    Verify {
        /// Book title to verify (omit and use --all to verify everything)
        book: Option<String>,

        /// Verify files for every book in the library
        #[arg(long)]
        all: bool,
    },

    /// Report disk usage of associated files, with an optional breakdown
    ///
    /// Totals reflect the catalog's recorded file sizes. Use --by to group the
    /// breakdown by format (default), author, or shelf. A file linked to
    /// multiple authors or shelves is counted under each of them.
    Usage {
        /// Group the breakdown by: format (default), author, or shelf
        #[arg(long, value_name = "DIMENSION")]
        by: Option<String>,
    },
}

#[derive(Subcommand)]
enum BulkAction {
    /// Add a tag to all books matching a filter
    Tag {
        /// Tag name to apply
        tag: String,

        /// Filter by reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by existing tag
        #[arg(long)]
        existing_tag: Option<String>,

        /// Preview changes without applying
        #[arg(long)]
        dry_run: bool,
    },

    /// Change reading status for all books matching a filter
    Status {
        /// New reading status (want-to-read, reading, read, abandoned, on-hold)
        new_status: String,

        /// Filter by current reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Preview changes without applying
        #[arg(long)]
        dry_run: bool,
    },

    /// Delete all books matching a filter
    Delete {
        /// Filter by reading status
        #[arg(long, short)]
        status: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Preview what would be deleted without actually deleting
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum WorkAction {
    /// Link a book to a work (creates the work if needed)
    Link {
        /// Book title or ID
        book: String,

        /// Work title (creates or finds an existing work)
        #[arg(long)]
        work: String,
    },

    /// Unlink a book from its work
    Unlink {
        /// Book title or ID
        book: String,
    },

    /// Show all editions of a work
    Show {
        /// Work title to display
        work: String,
    },

    /// Auto-detect potential work groups from ungrouped books
    Auto,
}

#[derive(Subcommand)]
enum ShelfAction {
    /// Create a shelf (use --smart --filter for smart shelves)
    Create {
        /// Shelf name
        name: String,

        /// Create a smart shelf with a filter rule
        #[arg(long)]
        smart: bool,

        /// Filter expression (required with --smart)
        #[arg(long, requires = "smart")]
        filter: Option<String>,
    },

    /// List all shelves
    List,

    /// Show books in a shelf (smart shelves are evaluated dynamically)
    Show {
        /// Shelf name
        name: String,
    },

    /// Delete a shelf
    Delete {
        /// Shelf name
        name: String,
    },

    /// Add a book to a regular shelf
    Add {
        /// Shelf name
        shelf: String,

        /// Book title or ID
        book: String,
    },

    /// Remove a book from a regular shelf
    Remove {
        /// Shelf name
        shelf: String,

        /// Book title or ID
        book: String,
    },
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

        /// Encrypt the backup (AES-256-GCM). A sync-enrolled device seals it with
        /// the library data key; an offline-only device prompts for a passphrase
        /// (or reads TOKU_BACKUP_PASSPHRASE) and embeds the KDF salt so the
        /// archive restores anywhere with the passphrase. Default: unencrypted.
        #[arg(long)]
        encrypt: bool,
    },
}

#[derive(Clone, Subcommand)]
enum SyncAction {
    /// [Deprecated] Set up sync with a per-library passphrase. Prefer `signup`/`login`.
    Init {
        /// Sync server URL (default: http://localhost:8080)
        #[arg(long, default_value = "http://localhost:8080")]
        server: String,

        /// Library ID (UUID — use the same ID across all devices for one library)
        #[arg(long)]
        library_id: Option<String>,

        /// Name for this device (defaults to hostname)
        #[arg(long)]
        device_name: Option<String>,

        /// Deprecated: client-side encryption is now mandatory and you are
        /// always prompted for a passphrase. This flag is kept for
        /// compatibility and has no effect.
        #[arg(long, hide = true)]
        passphrase: bool,
    },

    /// Create an account on a sync server (generates your Secret Key + Emergency Kit)
    Signup {
        /// Sync server URL (default: http://localhost:8080)
        #[arg(long, default_value = "http://localhost:8080")]
        server: String,

        /// Account email
        #[arg(long)]
        email: Option<String>,

        /// Name for this device (defaults to hostname)
        #[arg(long)]
        device_name: Option<String>,

        /// Write the Emergency Kit to a file (PDF if the path ends in .pdf,
        /// HTML for .html, otherwise plain text). Defaults to printing the kit.
        #[arg(long)]
        kit_out: Option<PathBuf>,
    },

    /// Log in to your account on this (already-enrolled) device
    Login {
        /// Sync server URL (default: http://localhost:8080)
        #[arg(long, default_value = "http://localhost:8080")]
        server: String,

        /// Account email
        #[arg(long)]
        email: Option<String>,
    },

    /// Enroll this new device into an existing account (password + Secret Key)
    Enroll {
        /// Sync server URL (default: http://localhost:8080)
        #[arg(long, default_value = "http://localhost:8080")]
        server: String,

        /// Account email
        #[arg(long)]
        email: Option<String>,

        /// Existing library ID to join (omit to create a fresh library)
        #[arg(long)]
        library_id: Option<String>,

        /// Name for this device (defaults to hostname)
        #[arg(long)]
        device_name: Option<String>,
    },

    /// Show sync status
    Status,

    /// Push local changes to the sync server
    Push,

    /// Pull remote changes from the sync server
    Pull,

    /// Restore this device's library from the server (new-device provisioning /
    /// recovery): download and apply the latest snapshot, then pull remaining ops
    Bootstrap {
        /// Discard the local pull cursor and re-sync from scratch (full re-download)
        #[arg(long)]
        reset_cursor: bool,
    },

    /// List all devices registered to this library
    Devices,

    /// Deregister another device from the sync server
    Deregister {
        /// Device ID to deregister
        device_id: String,
    },

    /// Disable sync (local data preserved)
    Disable,

    /// Purge tombstoned books older than the retention period
    Purge {
        /// Retention period in days (default: 30)
        #[arg(long, default_value = "30")]
        days: i64,
    },

    /// Change the sync encryption passphrase and re-encrypt all server ops
    Rekey,

    /// Compact the op log by creating a snapshot and pruning old ops
    Compact,

    /// One-time upgrade from the legacy relay (single-passphrase) model to an
    /// account: generates a Secret Key + account, re-protects all server data
    /// under the new zero-knowledge key hierarchy, and closes legacy access.
    Migrate {
        /// Account email
        #[arg(long)]
        email: Option<String>,

        /// Write the Emergency Kit to a file (PDF if the path ends in .pdf,
        /// HTML for .html, otherwise plain text). Defaults to printing the kit.
        #[arg(long)]
        kit_out: Option<PathBuf>,
    },

    /// Review and resolve sync conflicts (note/review edits that collided across devices)
    Conflicts {
        #[command(subcommand)]
        action: Option<ConflictAction>,
    },
}

#[derive(Clone, Subcommand)]
enum AccountAction {
    /// Manage your account Secret Key
    SecretKey {
        #[command(subcommand)]
        action: SecretKeyAction,
    },

    /// Generate and render an Emergency Kit (account details + Secret Key)
    EmergencyKit {
        /// Account email / identifier
        #[arg(long)]
        email: Option<String>,

        /// Sync server URL
        #[arg(long)]
        server: Option<String>,

        /// Use an existing Secret Key (otherwise a fresh one is generated)
        #[arg(long)]
        secret_key: Option<String>,

        /// Kit output format
        #[arg(long, value_enum, default_value = "text")]
        kit_format: KitFormat,

        /// Write the kit to a file instead of stdout (required for PDF)
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}

#[derive(Clone, Subcommand)]
enum SecretKeyAction {
    /// Generate a new Secret Key
    Generate {
        /// Write the formatted Secret Key to a file instead of stdout
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum KitFormat {
    /// Plain text
    Text,
    /// Printable, self-contained HTML
    Html,
    /// PDF document
    Pdf,
}

#[derive(Clone, Subcommand)]
enum ConflictAction {
    /// List unresolved conflicts (default when no subcommand is given)
    List,

    /// Show the local/remote diff for a single conflict
    Show {
        /// Conflict ID
        id: String,
    },

    /// Resolve a single conflict, keeping one side or a custom merged value
    Resolve {
        /// Conflict ID
        id: String,

        /// Which side to keep (mutually exclusive with --value)
        #[arg(long, value_enum, group = "resolution")]
        keep: Option<KeepArg>,

        /// Resolve with a custom merged value (mutually exclusive with --keep)
        #[arg(long, group = "resolution")]
        value: Option<String>,
    },

    /// Resolve every unresolved conflict the same way
    ResolveAll {
        /// Which side to keep
        #[arg(long, value_enum)]
        keep: KeepArg,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum KeepArg {
    /// Keep this device's local value
    Local,
    /// Keep the incoming remote value
    Remote,
}

impl From<KeepArg> for toku_db::ConflictKeep {
    fn from(value: KeepArg) -> Self {
        match value {
            KeepArg::Local => toku_db::ConflictKeep::Local,
            KeepArg::Remote => toku_db::ConflictKeep::Remote,
        }
    }
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
        Commands::Serve {
            port,
            host,
            hosted,
            insecure_cookies,
        } => {
            let db_path = data_dir.join("toku.db");
            let mode = if *hosted {
                toku_web::WebMode::Hosted
            } else {
                toku_web::WebMode::Local
            };
            // Hosted mode marks cookies Secure by default (TLS expected, e.g. via
            // a reverse proxy); --insecure-cookies opts out for local testing.
            let secure_cookies = *hosted && !*insecure_cookies;
            let host = host.clone();
            let port = *port;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            return rt.block_on(async move {
                toku_web::serve(db_path, &host, port, mode, secure_cookies)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            });
        }
        Commands::Opds { action } => {
            return cmd_opds(&data_dir, action);
        }
        Commands::Sync { action } => {
            return cmd_sync(&data_dir, action.clone(), &cli.format);
        }
        Commands::Account { action } => {
            return cmd_account(action.clone());
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
            tag,
            mood,
            pace,
            content_warning,
            status,
        } => cmd_add(
            &repo,
            &db_path,
            title,
            author,
            isbn,
            &book_format,
            &tag,
            &mood,
            pace.as_deref(),
            &content_warning,
            status.as_deref(),
            &cli.format,
        ),
        Commands::Show { query } => cmd_show(&repo, &query, &cli.format),
        Commands::Edit {
            query,
            mood,
            pace,
            content_warning,
            remove_mood,
            remove_content_warning,
            rating,
        } => cmd_edit(
            &repo,
            &query,
            &mood,
            pace.as_deref(),
            &content_warning,
            &remove_mood,
            &remove_content_warning,
            rating,
            &cli.format,
        ),
        Commands::List {
            status,
            tag,
            mood,
            pace,
        } => cmd_list(
            &repo,
            status.as_deref(),
            tag.as_deref(),
            &mood,
            pace.as_deref(),
            &cli.format,
        ),
        Commands::Search { query, status, tag } => cmd_search(
            &repo,
            &query,
            status.as_deref(),
            tag.as_deref(),
            &cli.format,
        ),
        Commands::Import { source } => cmd_import(&db, &repo, source, &data_dir, &cli.format),
        Commands::Lookup { query, limit } => cmd_lookup(&query, limit, &cli.format),
        Commands::Reading { action } => cmd_reading(&repo, action, &cli.format),
        Commands::Tag { action } => cmd_tag(&repo, action, &cli.format),
        Commands::File { action } => cmd_file(&db, &repo, action, &data_dir, &cli.format),
        Commands::Convert {
            book,
            to,
            from,
            force,
        } => cmd_convert(&db, &repo, &book, &to, from.as_deref(), force),
        Commands::Export { target } => cmd_export(&db, &data_dir, target),
        Commands::Bulk { action } => cmd_bulk(&repo, action),
        Commands::Stats {
            year,
            author,
            mood_trends,
        } => cmd_stats(&repo, year, author.as_deref(), mood_trends, &cli.format),
        Commands::Work { action } => cmd_work(&repo, action, &cli.format),
        Commands::Merge { keep, remove } => cmd_merge(&repo, &keep, &remove, &cli.format),
        Commands::Shelf { action } => cmd_shelf(&repo, action, &cli.format),
        Commands::Config { .. }
        | Commands::Completions { .. }
        | Commands::Serve { .. }
        | Commands::Opds { .. }
        | Commands::Sync { .. }
        | Commands::Account { .. } => {
            unreachable!()
        }
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

/// Handle `toku opds` — serve the OPDS catalog or manage its auth credentials.
fn cmd_opds(data_dir: &Path, action: &OpdsAction) -> Result<()> {
    match action {
        OpdsAction::Serve { port, host } => {
            let config = TokuConfig::load(data_dir).context("failed to load config")?;
            let auth = if config.opds.auth_enabled() {
                Some(config.opds.clone())
            } else {
                None
            };
            let db_path = data_dir.join("toku.db");
            let host = host.clone();
            let port = *port;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(async move {
                toku_web::serve_opds(db_path, &host, port, auth)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            })
        }
        OpdsAction::SetPassword { username } => {
            let username = username.trim();
            if username.is_empty() {
                anyhow::bail!("username cannot be empty");
            }
            let password = read_password_prompt("OPDS password: ")?;
            if password.is_empty() {
                anyhow::bail!("password cannot be empty");
            }
            let confirm = read_password_prompt("Confirm OPDS password: ")?;
            if password != confirm {
                anyhow::bail!("passwords do not match");
            }
            let mut config = TokuConfig::load(data_dir).context("failed to load config")?;
            config.opds.username = Some(username.to_string());
            config.opds.password_hash = Some(toku_core::OpdsConfig::hash_password(&password));
            config.save(data_dir).context("failed to save config")?;
            eprintln!("OPDS HTTP Basic auth enabled for user '{username}'.");
            eprintln!(
                "The catalog will now require this login. Restart `toku opds serve` to apply."
            );
            Ok(())
        }
        OpdsAction::Disable => {
            let mut config = TokuConfig::load(data_dir).context("failed to load config")?;
            let was_enabled = config.opds.auth_enabled();
            config.opds.username = None;
            config.opds.password_hash = None;
            config.save(data_dir).context("failed to save config")?;
            if was_enabled {
                eprintln!("OPDS HTTP Basic auth disabled. Restart `toku opds serve` to apply.");
            } else {
                eprintln!("OPDS HTTP Basic auth was not enabled.");
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_add(
    repo: &BookRepository,
    db_path: &Path,
    title: Option<String>,
    author: Option<String>,
    isbn: Option<String>,
    book_format: &str,
    tags: &[String],
    moods: &[String],
    pace: Option<&str>,
    content_warnings: &[String],
    initial_status: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    let format: BookFormat = book_format
        .parse()
        .with_context(|| format!("invalid format: {book_format}"))?;

    // Parse status early to fail fast before creating the book
    let target_status = initial_status
        .map(|s| ReadingStatus::from_str(s).map_err(|_| anyhow::anyhow!("invalid status: {s}")))
        .transpose()?;

    if let Some(isbn_str) = &isbn {
        let validated = Isbn::parse(isbn_str).context("invalid ISBN")?;
        let isbn13 = validated.to_isbn13();

        // Check for existing book with this ISBN
        if let Some(existing) = repo.find_by_isbn(&isbn13)? {
            // Apply tags/status to existing book instead of silently ignoring
            apply_post_add(
                repo,
                &existing,
                tags,
                moods,
                pace,
                content_warnings,
                target_status,
            )?;
            print_books(&[existing], repo, output_format)?;
            eprintln!("Book already exists — applied tags/status updates");
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

                apply_post_add(
                    repo,
                    &book,
                    tags,
                    moods,
                    pace,
                    content_warnings,
                    target_status,
                )?;
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
                apply_post_add(
                    repo,
                    &book,
                    tags,
                    moods,
                    pace,
                    content_warnings,
                    target_status,
                )?;
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

        apply_post_add(
            repo,
            &book,
            tags,
            moods,
            pace,
            content_warnings,
            target_status,
        )?;
        print_books(&[book], repo, output_format)?;
        eprintln!("✓ Added manually");
    } else {
        anyhow::bail!("provide --isbn or --title to add a book");
    }

    Ok(())
}

/// Apply tags, mood tags, pace, content warnings, and optional status change after creating/finding a book.
#[allow(clippy::too_many_arguments)]
fn apply_post_add(
    repo: &BookRepository,
    book: &Book,
    tags: &[String],
    moods: &[String],
    pace: Option<&str>,
    content_warnings: &[String],
    status: Option<ReadingStatus>,
) -> Result<()> {
    for tag_name in tags {
        let trimmed = tag_name.trim();
        if !trimmed.is_empty() {
            repo.add_tag_to_book(&book.id, trimmed)?;
        }
    }
    for mood in moods {
        let trimmed = mood.trim();
        if !trimmed.is_empty() {
            repo.add_typed_tag_to_book(&book.id, trimmed, TagType::Mood)?;
        }
    }
    if let Some(pace_str) = pace {
        let pace_rating: PaceRating = pace_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid pace: {pace_str} (use fast, medium, or slow)"))?;
        repo.set_book_pace(&book.id, pace_rating)?;
    }
    for cw in content_warnings {
        let trimmed = cw.trim();
        if !trimmed.is_empty() {
            repo.add_typed_tag_to_book(&book.id, trimmed, TagType::ContentWarning)?;
        }
    }
    if let Some(target) = status {
        repo.update_book_status(&book.id, target)?;
        if target == ReadingStatus::Reading {
            let session = ReadingSession::new(book.id);
            repo.create_reading_session(&session)?;
        }
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

#[allow(clippy::too_many_arguments)]
fn cmd_edit(
    repo: &BookRepository,
    query: &str,
    moods: &[String],
    pace: Option<&str>,
    content_warnings: &[String],
    remove_moods: &[String],
    remove_content_warnings: &[String],
    rating: Option<i32>,
    output_format: &OutputFormat,
) -> Result<()> {
    let book = resolve_book(repo, query)?;
    let mut changes = Vec::new();

    // Add mood tags
    for mood in moods {
        let trimmed = mood.trim();
        if !trimmed.is_empty() {
            repo.add_typed_tag_to_book(&book.id, trimmed, TagType::Mood)?;
            changes.push(format!("added mood \"{trimmed}\""));
        }
    }

    // Remove mood tags
    for mood in remove_moods {
        let trimmed = mood.trim();
        if !trimmed.is_empty() {
            repo.remove_typed_tag_from_book(&book.id, trimmed, TagType::Mood)?;
            changes.push(format!("removed mood \"{trimmed}\""));
        }
    }

    // Set pace
    if let Some(pace_str) = pace {
        let pace_rating: PaceRating = pace_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid pace: {pace_str} (use fast, medium, or slow)"))?;
        repo.set_book_pace(&book.id, pace_rating)?;
        changes.push(format!("set pace to {pace_str}"));
    }

    // Add content warnings
    for cw in content_warnings {
        let trimmed = cw.trim();
        if !trimmed.is_empty() {
            repo.add_typed_tag_to_book(&book.id, trimmed, TagType::ContentWarning)?;
            changes.push(format!("added content warning \"{trimmed}\""));
        }
    }

    // Remove content warnings
    for cw in remove_content_warnings {
        let trimmed = cw.trim();
        if !trimmed.is_empty() {
            repo.remove_typed_tag_from_book(&book.id, trimmed, TagType::ContentWarning)?;
            changes.push(format!("removed content warning \"{trimmed}\""));
        }
    }

    // Set rating
    if let Some(r) = rating {
        if !(0..=10).contains(&r) {
            anyhow::bail!("rating must be between 0 and 10, got {r}");
        }
        repo.update_book_rating(&book.id, r)?;
        changes.push(format!("set rating to {:.1}★", r as f32 / 2.0));
    }

    if changes.is_empty() {
        eprintln!("No changes specified for \"{}\"", book.title);
        eprintln!(
            "Use --mood, --pace, --content-warning, --remove-mood, --remove-content-warning, or --rating"
        );
    } else {
        // Re-fetch to show updated rating, then display
        let updated = repo.get_book(&book.id).unwrap_or(book.clone());
        print_book_detail(&updated, repo, output_format)?;
        for change in &changes {
            eprintln!("  ✓ {change}");
        }
        eprintln!("✓ Updated \"{}\" ({} change(s))", book.title, changes.len());
    }

    Ok(())
}

fn cmd_lookup(query: &str, limit: usize, output_format: &OutputFormat) -> Result<()> {
    eprintln!("Searching Open Library for \"{query}\"...\n");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let results = rt
        .block_on(toku_meta::search_books(query, limit))
        .context("Open Library search failed")?;

    if results.is_empty() {
        eprintln!("No results found.");
        return Ok(());
    }

    match output_format {
        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "title": r.title,
                        "authors": r.authors,
                        "year": r.first_publish_year,
                        "isbn": r.isbn,
                        "pages": r.page_count,
                        "editions": r.edition_count,
                        "languages": r.languages,
                        "openlibrary_key": r.openlibrary_key,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json_results)?);
        }
        OutputFormat::Csv => {
            let mut wtr = csv::Writer::from_writer(std::io::stdout());
            wtr.write_record(["Title", "Author", "Year", "ISBN", "Pages", "Editions"])?;
            for r in &results {
                wtr.write_record([
                    &r.title,
                    &r.authors.join(", "),
                    &r.first_publish_year
                        .map_or("—".to_string(), |y| y.to_string()),
                    r.isbn.as_deref().unwrap_or("—"),
                    &r.page_count.map_or("—".to_string(), |p| p.to_string()),
                    &r.edition_count.to_string(),
                ])?;
            }
            wtr.flush()?;
        }
        OutputFormat::Table => {
            use tabled::settings::Style;
            use tabled::{Table, Tabled};

            let term_width = crossterm::terminal::size()
                .map(|(w, _)| w as usize)
                .unwrap_or(120);

            // #(3) + Title + Author + Year(4) + ISBN(13) + Pages(5) + Editions(4)
            let fixed_overhead = 50;
            let flexible = term_width.saturating_sub(fixed_overhead);
            let title_max = (flexible * 3 / 5).max(15);
            let author_max = flexible.saturating_sub(title_max).max(10);

            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "#")]
                idx: String,
                #[tabled(rename = "Title")]
                title: String,
                #[tabled(rename = "Author")]
                author: String,
                #[tabled(rename = "Year")]
                year: String,
                #[tabled(rename = "ISBN")]
                isbn: String,
                #[tabled(rename = "Pages")]
                pages: String,
                #[tabled(rename = "Ed.")]
                editions: String,
            }

            let rows: Vec<Row> = results
                .iter()
                .enumerate()
                .map(|(i, r)| Row {
                    idx: format!("{}", i + 1),
                    title: import_ui::truncate_str(&r.title, title_max),
                    author: import_ui::truncate_str(&r.authors.join(", "), author_max),
                    year: r
                        .first_publish_year
                        .map_or("—".to_string(), |y| y.to_string()),
                    isbn: r.isbn.clone().unwrap_or_else(|| "—".to_string()),
                    pages: r.page_count.map_or("—".to_string(), |p| p.to_string()),
                    editions: r.edition_count.to_string(),
                })
                .collect();

            let mut table = Table::new(rows);
            table.with(Style::rounded());
            println!("{table}");

            eprintln!("\nTo add a book: toku add --isbn <ISBN>",);
        }
    }

    Ok(())
}

fn cmd_list(
    repo: &BookRepository,
    status: Option<&str>,
    tag: Option<&str>,
    moods: &[String],
    pace: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    let books = if let Some(tag_name) = tag {
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

    // Apply mood filter (same-type OR): keep books that have ANY of the specified moods
    let filtered = if moods.is_empty() {
        filtered
    } else {
        let mut mood_matched: Vec<Book> = Vec::new();
        for book in filtered {
            let book_moods = repo.get_book_tags_by_type(&book.id, TagType::Mood)?;
            let mood_names: Vec<String> = book_moods.iter().map(|t| t.name.clone()).collect();
            if moods.iter().any(|m| mood_names.contains(m)) {
                mood_matched.push(book);
            }
        }
        mood_matched
    };

    // Apply pace filter (AND with mood)
    let filtered = if let Some(pace_str) = pace {
        let mut pace_matched: Vec<Book> = Vec::new();
        for book in filtered {
            let book_pace = repo.get_book_tags_by_type(&book.id, TagType::Pace)?;
            let pace_names: Vec<String> = book_pace.iter().map(|t| t.name.clone()).collect();
            if pace_names.contains(&pace_str.to_string()) {
                pace_matched.push(book);
            }
        }
        pace_matched
    } else {
        filtered
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
    tag: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    let books = repo.search_books_filtered(query, status, None, tag)?;
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
    // Gather typed tags for all output formats
    let mood_tags = repo
        .get_book_tags_by_type(&book.id, TagType::Mood)
        .unwrap_or_default();
    let pace_tags = repo
        .get_book_tags_by_type(&book.id, TagType::Pace)
        .unwrap_or_default();
    let cw_tags = repo
        .get_book_tags_by_type(&book.id, TagType::ContentWarning)
        .unwrap_or_default();

    match output_format {
        OutputFormat::Json => {
            let authors: Vec<String> = repo
                .get_book_authors(&book.id)
                .unwrap_or_default()
                .into_iter()
                .map(|(a, _)| a.name)
                .collect();
            let general_tags = repo
                .get_book_tags_by_type(&book.id, TagType::General)
                .unwrap_or_default();
            let json = serde_json::json!({
                "id": book.id.to_string(),
                "title": book.title,
                "authors": authors,
                "status": book.status.as_str(),
                "format": book.format.as_str(),
                "rating": book.rating,
                "pages": book.page_count,
                "work_id": book.work_id.map(|w| w.to_string()),
                "tags": general_tags.iter().map(|t| &t.name).collect::<Vec<_>>(),
                "moods": mood_tags.iter().map(|t| &t.name).collect::<Vec<_>>(),
                "pace": pace_tags.first().map(|t| &t.name),
                "content_warnings": cw_tags.iter().map(|t| &t.name).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
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
            if !mood_tags.is_empty() {
                let names: Vec<&str> = mood_tags.iter().map(|t| t.name.as_str()).collect();
                println!("  Moods:   {}", names.join(", "));
            }
            if let Some(pace) = pace_tags.first() {
                println!("  Pace:    {}", pace.name);
            }
            if !cw_tags.is_empty() {
                let names: Vec<&str> = cw_tags.iter().map(|t| t.name.as_str()).collect();
                println!("  ⚠ CW:    {}", names.join(", "));
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
            if let Some(work_id) = &book.work_id {
                if let Ok(work) = repo.get_work(work_id) {
                    println!("  Work:    {} ({})", work.title, work_id);
                } else {
                    println!("  Work:    {work_id}");
                }
            }
        }
    }
    Ok(())
}

fn cmd_import(
    db: &Database,
    _repo: &BookRepository,
    source: ImportSource,
    data_dir: &Path,
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
        ImportSource::Storygraph { path, dry_run } => {
            import_ui::run_storygraph_import(db, &path, dry_run, output_format)
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
        ImportSource::Backup {
            path,
            replace,
            dry_run,
        } => cmd_import_backup(db, &path, replace, dry_run, data_dir, output_format),
    }
}

fn cmd_import_backup(
    db: &Database,
    path: &Path,
    replace: bool,
    dry_run: bool,
    data_dir: &Path,
    output_format: &OutputFormat,
) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("backup not found: {}", path.display());
    }

    let manifest =
        toku_export::read_backup_manifest(path).context("could not read backup manifest")?;

    if dry_run {
        let c = &manifest.counts;
        match output_format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&manifest)?);
            }
            _ => {
                eprintln!("Dry run — no changes will be made.");
                eprintln!(
                    "Backup format v{} ({}), created {}",
                    manifest.format_version,
                    if manifest.encrypted {
                        "encrypted"
                    } else {
                        "plaintext"
                    },
                    manifest.created_at
                );
                eprintln!(
                    "Would restore: {} books, {} reading sessions, {} progress, {} notes, \
                     {} reviews, {} tags, {} shelves, {} works, {} series, {} files",
                    c.books,
                    c.reading_sessions,
                    c.reading_progress,
                    c.notes,
                    c.reviews,
                    c.tags,
                    c.shelves,
                    c.works,
                    c.series,
                    c.files,
                );
                eprintln!("Mode: {}", if replace { "replace" } else { "merge" });
            }
        }
        return Ok(());
    }

    // Decrypt only when the artifact is sealed; plaintext restores stay fully
    // offline with no key required. A passphrase-sealed backup (manifest carries
    // a `kdf` descriptor) re-derives its key from the embedded salt + passphrase;
    // a sync-key backup uses the enrolled library key as before.
    let key = if manifest.encrypted {
        if let Some(kdf) = manifest.kdf.as_ref() {
            let passphrase = read_backup_passphrase_open()?;
            Some(
                kdf.derive_key(&passphrase)
                    .map_err(|e| anyhow::anyhow!("failed to derive backup key: {e}"))?,
            )
        } else {
            Some(load_library_key(data_dir).context(
                "backup is encrypted but no library data key is configured; enroll this device \
                 with `toku sync` to restore an encrypted backup",
            )?)
        }
    } else {
        None
    };

    let mode = if replace {
        toku_db::RestoreMode::Replace
    } else {
        toku_db::RestoreMode::Merge
    };

    let result =
        toku_export::import_backup(path, db, data_dir, mode, key.as_ref()).map_err(|e| {
            if manifest.kdf.is_some() && matches!(e, toku_export::ExportError::Crypto(_)) {
                anyhow::anyhow!("could not decrypt backup — wrong passphrase or corrupted archive")
            } else {
                anyhow::anyhow!("backup restore failed: {e}")
            }
        })?;

    match output_format {
        OutputFormat::Json => {
            let out = serde_json::json!({
                "mode": if replace { "replace" } else { "merge" },
                "books_inserted": result.books_inserted,
                "books_updated": result.books_updated,
                "reading_sessions": result.reading_sessions,
                "reading_progress": result.reading_progress,
                "notes": result.notes,
                "reviews": result.reviews,
                "tags": result.tags,
                "shelves": result.shelves,
                "works": result.works,
                "series": result.series,
                "isbns": result.isbns,
                "files": result.files,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        OutputFormat::Csv => {
            println!("metric,count");
            println!("books_inserted,{}", result.books_inserted);
            println!("books_updated,{}", result.books_updated);
            println!("reading_sessions,{}", result.reading_sessions);
            println!("reading_progress,{}", result.reading_progress);
            println!("notes,{}", result.notes);
            println!("reviews,{}", result.reviews);
            println!("tags,{}", result.tags);
            println!("shelves,{}", result.shelves);
            println!("works,{}", result.works);
            println!("series,{}", result.series);
            println!("isbns,{}", result.isbns);
            println!("files,{}", result.files);
        }
        OutputFormat::Table => {
            eprintln!(
                "✓ Restored backup ({} mode)",
                if replace { "replace" } else { "merge" }
            );
            eprintln!(
                "  {} books added, {} updated · {} sessions · {} progress · {} notes · \
                 {} reviews · {} files",
                result.books_inserted,
                result.books_updated,
                result.reading_sessions,
                result.reading_progress,
                result.notes,
                result.reviews,
                result.files,
            );
        }
    }

    Ok(())
}

/// Load the library data key from the configured sync server's local key store,
/// if one is enrolled. Fully local — no network access.
fn load_library_key(data_dir: &Path) -> Result<toku_core::SyncKey> {
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let server = config
        .sync
        .as_ref()
        .map(|s| s.server.clone())
        .ok_or_else(|| anyhow::anyhow!("no sync server configured"))?;
    let token_store = sync::token_store::TokenStore::new(data_dir);
    let key_bytes = token_store
        .load_sync_key(&server)?
        .ok_or_else(|| anyhow::anyhow!("no library data key stored for {server}"))?;
    toku_core::SyncKey::from_exported_bytes(&key_bytes)
        .map_err(|e| anyhow::anyhow!("stored library data key is invalid: {e}"))
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
        ExportTarget::Backup { output, encrypt } => {
            if !encrypt {
                toku_export::export_backup(db, data_dir, &output, None)
                    .context("backup export failed")?;
                eprintln!("✓ Backup saved to {}", output.display());
                return Ok(());
            }

            // Encrypted path. A sync user keeps today's behavior (seal with the
            // enrolled library key). An offline-only user (no sync configured)
            // falls back to a passphrase-derived local key; the KDF salt travels
            // in the archive so the backup restores anywhere with the passphrase.
            if sync_is_configured(data_dir) {
                let key = load_library_key(data_dir).context(
                    "cannot encrypt backup: enroll this device with `toku sync` first, \
                     or omit --encrypt for a plaintext backup",
                )?;
                toku_export::export_backup(db, data_dir, &output, Some(&key))
                    .context("backup export failed")?;
                eprintln!(
                    "✓ Backup saved to {} (encrypted with sync library key)",
                    output.display()
                );
            } else {
                let passphrase = read_backup_passphrase_new()?;
                let kdf = toku_core::backup_schema::BackupKdf::generate()
                    .map_err(|e| anyhow::anyhow!("failed to prepare backup key: {e}"))?;
                let key = kdf
                    .derive_key(&passphrase)
                    .map_err(|e| anyhow::anyhow!("failed to derive backup key: {e}"))?;
                toku_export::export_backup_with_kdf(db, data_dir, &output, &key, kdf)
                    .context("backup export failed")?;
                eprintln!(
                    "✓ Backup saved to {} (encrypted with passphrase)",
                    output.display()
                );
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
                        tag_type: String,
                        books: i64,
                    }
                    let out: Vec<TagOut> = tags
                        .iter()
                        .map(|(t, count)| TagOut {
                            name: t.name.clone(),
                            tag_type: t.tag_type.to_string(),
                            books: *count,
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Csv => {
                    println!("tag,type,books");
                    for (t, count) in &tags {
                        println!(
                            "\"{}\",{},{}",
                            t.name.replace('"', "\"\""),
                            t.tag_type,
                            count
                        );
                    }
                }
                OutputFormat::Table => {
                    use tabled::{Table, Tabled};

                    #[derive(Tabled)]
                    struct Row {
                        #[tabled(rename = "Tag")]
                        name: String,
                        #[tabled(rename = "Type")]
                        tag_type: String,
                        #[tabled(rename = "Books")]
                        books: i64,
                    }

                    let rows: Vec<Row> = tags
                        .iter()
                        .map(|(t, count)| Row {
                            name: t.name.clone(),
                            tag_type: t.tag_type.to_string(),
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

fn human_size(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn cmd_file(
    db: &Database,
    repo: &BookRepository,
    action: FileAction,
    data_dir: &Path,
    output_format: &OutputFormat,
) -> Result<()> {
    let files = FileRepository::new(db);
    match action {
        FileAction::Add { book, path } => {
            let book = resolve_book(repo, &book)?;
            if !path.is_file() {
                anyhow::bail!("file not found: {}", path.display());
            }
            let format = FileFormat::from_path(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
            let size_bytes = std::fs::metadata(&path)
                .with_context(|| format!("reading {}", path.display()))?
                .len() as i64;
            let checksum = sha256_file(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
            let stored = path
                .canonicalize()
                .unwrap_or(path.clone())
                .to_string_lossy()
                .to_string();
            let file = EbookFile::new(book.id, stored, format, size_bytes, checksum);
            files.add_file(&file).map_err(|e| anyhow::anyhow!("{e}"))?;
            eprintln!(
                "✓ Linked {} ({}) to \"{}\"",
                file.format,
                human_size(file.size_bytes),
                book.title
            );
            Ok(())
        }
        FileAction::List { book } => {
            let book = resolve_book(repo, &book)?;
            let list = files
                .list_files(&book.id)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if list.is_empty() {
                eprintln!(
                    "No files linked to \"{}\". Add one with: toku file add",
                    book.title
                );
                return Ok(());
            }
            match output_format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&list)?);
                }
                OutputFormat::Csv => {
                    println!("format,size_bytes,checksum,exists,path");
                    for f in &list {
                        println!(
                            "{},{},{},{},\"{}\"",
                            f.format,
                            f.size_bytes,
                            f.checksum,
                            Path::new(&f.path).exists(),
                            f.path.replace('"', "\"\"")
                        );
                    }
                }
                OutputFormat::Table => {
                    use tabled::{Table, Tabled};

                    #[derive(Tabled)]
                    struct Row {
                        #[tabled(rename = "Format")]
                        format: String,
                        #[tabled(rename = "Size")]
                        size: String,
                        #[tabled(rename = "Checksum")]
                        checksum: String,
                        #[tabled(rename = "On disk")]
                        exists: String,
                        #[tabled(rename = "Path")]
                        path: String,
                    }
                    let rows: Vec<Row> = list
                        .iter()
                        .map(|f| Row {
                            format: f.format.to_string(),
                            size: human_size(f.size_bytes),
                            checksum: f.checksum.chars().take(12).collect(),
                            exists: if Path::new(&f.path).exists() {
                                "yes"
                            } else {
                                "MISSING"
                            }
                            .to_string(),
                            path: f.path.clone(),
                        })
                        .collect();
                    println!("{}", Table::new(rows));
                }
            }
            eprintln!("\n{} file(s)", list.len());
            Ok(())
        }
        FileAction::Remove {
            book,
            target,
            delete_file,
        } => {
            let book = resolve_book(repo, &book)?;
            let removed = match target.parse::<FileFormat>() {
                Ok(fmt) => files.remove_by_format(&book.id, fmt),
                Err(_) => files.remove_by_path(&book.id, &target),
            }
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            match removed {
                Some(f) => {
                    if delete_file {
                        match std::fs::remove_file(&f.path) {
                            Ok(()) => eprintln!("✓ Deleted file {}", f.path),
                            Err(e) => eprintln!("⚠ Removed record, but file delete failed: {e}"),
                        }
                    }
                    eprintln!("✓ Removed {} from \"{}\"", f.format, book.title);
                    Ok(())
                }
                None => anyhow::bail!("no file matching \"{target}\" linked to \"{}\"", book.title),
            }
        }
        FileAction::Organize {
            book,
            all,
            dry_run,
            copy,
        } => cmd_file_organize(db, repo, data_dir, book, all, dry_run, copy, output_format),
        FileAction::Verify { book, all } => cmd_file_verify(db, repo, book, all, output_format),
        FileAction::Usage { by } => cmd_file_usage(db, repo, by.as_deref(), output_format),
    }
}

/// Recompute SHA-256 for associated files and report integrity problems.
///
/// Verifies a single book's files, or the whole library with `--all`. Prints a
/// per-file status plus a summary, and exits non-zero when any file is missing
/// or its contents no longer match the stored checksum.
fn cmd_file_verify(
    db: &Database,
    repo: &BookRepository,
    book: Option<String>,
    all: bool,
    output_format: &OutputFormat,
) -> Result<()> {
    let files = FileRepository::new(db);

    let targets: Vec<EbookFile> = match (&book, all) {
        (Some(_), true) => anyhow::bail!("pass either a book title or --all, not both"),
        (None, false) => anyhow::bail!("specify a book title, or use --all to verify everything"),
        (Some(title), false) => {
            let book = resolve_book(repo, title)?;
            files
                .list_files(&book.id)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
        (None, true) => files.list_all_files().map_err(|e| anyhow::anyhow!("{e}"))?,
    };

    if targets.is_empty() {
        eprintln!("No files to verify. Link some with: toku file add");
        return Ok(());
    }

    let outcomes: Vec<toku_files::VerifyOutcome> = targets
        .iter()
        .map(verify_file)
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mismatches = outcomes
        .iter()
        .filter(|o| o.status == VerifyStatus::Mismatch)
        .count();
    let missing = outcomes
        .iter()
        .filter(|o| o.status == VerifyStatus::Missing)
        .count();
    let ok = outcomes.len() - mismatches - missing;

    match output_format {
        OutputFormat::Json => {
            #[derive(serde::Serialize)]
            struct Row {
                status: String,
                format: String,
                path: String,
                stored_checksum: String,
                computed_checksum: Option<String>,
            }
            let rows: Vec<Row> = outcomes
                .iter()
                .map(|o| Row {
                    status: o.status.as_str().to_string(),
                    format: o.file.format.to_string(),
                    path: o.file.path.clone(),
                    stored_checksum: o.file.checksum.clone(),
                    computed_checksum: o.computed.clone(),
                })
                .collect();
            #[derive(serde::Serialize)]
            struct Report<'a> {
                ok: usize,
                mismatch: usize,
                missing: usize,
                files: &'a [Row],
            }
            let report = Report {
                ok,
                mismatch: mismatches,
                missing,
                files: &rows,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Csv => {
            println!("status,format,path,stored_checksum,computed_checksum");
            for o in &outcomes {
                println!(
                    "{},{},\"{}\",{},{}",
                    o.status,
                    o.file.format,
                    o.file.path.replace('"', "\"\""),
                    o.file.checksum,
                    o.computed.as_deref().unwrap_or("")
                );
            }
        }
        OutputFormat::Table => {
            use tabled::{Table, Tabled};

            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Status")]
                status: String,
                #[tabled(rename = "Format")]
                format: String,
                #[tabled(rename = "Path")]
                path: String,
            }
            let rows: Vec<Row> = outcomes
                .iter()
                .map(|o| Row {
                    status: match o.status {
                        VerifyStatus::Ok => "ok",
                        VerifyStatus::Mismatch => "MISMATCH",
                        VerifyStatus::Missing => "MISSING",
                    }
                    .to_string(),
                    format: o.file.format.to_string(),
                    path: o.file.path.clone(),
                })
                .collect();
            println!("{}", Table::new(rows));
        }
    }

    eprintln!(
        "\n{} ok, {} mismatch, {} missing ({} file(s) checked)",
        ok,
        mismatches,
        missing,
        outcomes.len()
    );

    if mismatches + missing > 0 {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::process::exit(1);
    }
    Ok(())
}

/// Report total disk usage of associated files with an optional breakdown.
///
/// Totals are computed from the recorded file sizes in the catalog. The
/// breakdown groups by format (default), author, or shelf; a file linked to
/// multiple authors or shelves contributes to each bucket, while books with
/// none fall into an `(unassigned)` bucket.
fn cmd_file_usage(
    db: &Database,
    repo: &BookRepository,
    by: Option<&str>,
    output_format: &OutputFormat,
) -> Result<()> {
    let files = FileRepository::new(db);
    let all_files = files.list_all_files().map_err(|e| anyhow::anyhow!("{e}"))?;

    let dimension = by.unwrap_or("format").to_lowercase();
    if !matches!(dimension.as_str(), "format" | "author" | "shelf") {
        anyhow::bail!("unknown --by value \"{dimension}\" (expected format, author, or shelf)");
    }

    let totals = usage_totals(&all_files);

    // Build the grouped breakdown. Author/shelf require joins through the book
    // repository; a file with no authors/shelves is bucketed as "(unassigned)".
    const UNASSIGNED: &str = "(unassigned)";
    let grouped: std::collections::BTreeMap<String, UsageTotals> = match dimension.as_str() {
        "format" => usage_by_key(&all_files, |f| vec![f.format.to_string()]),
        "author" => usage_by_key(&all_files, |f| {
            let names: Vec<String> = repo
                .get_book_authors(&f.book_id)
                .unwrap_or_default()
                .into_iter()
                .map(|(a, _)| a.name)
                .collect();
            if names.is_empty() {
                vec![UNASSIGNED.to_string()]
            } else {
                names
            }
        }),
        "shelf" => usage_by_key(&all_files, |f| {
            let names: Vec<String> = repo
                .get_book_shelves(&f.book_id)
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.name)
                .collect();
            if names.is_empty() {
                vec![UNASSIGNED.to_string()]
            } else {
                names
            }
        }),
        _ => unreachable!(),
    };

    // Sort breakdown rows by descending size for human-facing output.
    let mut rows: Vec<(String, UsageTotals)> = grouped.into_iter().collect();
    rows.sort_by(|a, b| {
        b.1.total_bytes
            .cmp(&a.1.total_bytes)
            .then_with(|| a.0.cmp(&b.0))
    });

    match output_format {
        OutputFormat::Json => {
            #[derive(serde::Serialize)]
            struct Group {
                key: String,
                file_count: u64,
                size_bytes: i64,
            }
            #[derive(serde::Serialize)]
            struct Report {
                dimension: String,
                total_files: u64,
                total_bytes: i64,
                breakdown: Vec<Group>,
            }
            let report = Report {
                dimension: dimension.clone(),
                total_files: totals.file_count,
                total_bytes: totals.total_bytes,
                breakdown: rows
                    .iter()
                    .map(|(k, v)| Group {
                        key: k.clone(),
                        file_count: v.file_count,
                        size_bytes: v.total_bytes,
                    })
                    .collect(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Csv => {
            println!("{dimension},file_count,size_bytes");
            for (k, v) in &rows {
                println!(
                    "\"{}\",{},{}",
                    k.replace('"', "\"\""),
                    v.file_count,
                    v.total_bytes
                );
            }
            println!("\"TOTAL\",{},{}", totals.file_count, totals.total_bytes);
        }
        OutputFormat::Table => {
            use tabled::{Table, Tabled};

            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Group")]
                key: String,
                #[tabled(rename = "Files")]
                files: u64,
                #[tabled(rename = "Size")]
                size: String,
            }
            let table_rows: Vec<Row> = rows
                .iter()
                .map(|(k, v)| Row {
                    key: k.clone(),
                    files: v.file_count,
                    size: human_size(v.total_bytes),
                })
                .collect();
            let header = match dimension.as_str() {
                "author" => "By author",
                "shelf" => "By shelf",
                _ => "By format",
            };
            println!("{header}:");
            println!("{}", Table::new(table_rows));
            println!(
                "\nTotal: {} across {} file(s)",
                human_size(totals.total_bytes),
                totals.file_count
            );
        }
    }
    Ok(())
}

/// Convert an associated ebook file to another format via Calibre's
/// `ebook-convert`, writing the output next to the source file and
/// auto-associating it with the book.
fn cmd_convert(
    db: &Database,
    repo: &BookRepository,
    book_query: &str,
    to: &str,
    from: Option<&str>,
    force: bool,
) -> Result<()> {
    let book = resolve_book(repo, book_query)?;
    let target_format: FileFormat = to.parse().map_err(|_| {
        anyhow::anyhow!("unsupported target format \"{to}\" (expected epub, pdf, mobi, or azw3)")
    })?;

    let files = FileRepository::new(db);
    let associated = files
        .list_files(&book.id)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if associated.is_empty() {
        anyhow::bail!(
            "No files linked to \"{}\". Add one first with: toku file add \"{}\" <path>",
            book.title,
            book.title
        );
    }

    // Pick the source file: honour --from, else auto-pick a single file, else
    // bail asking the user to disambiguate.
    let source = match from {
        Some(f) => {
            let fmt: FileFormat = f.parse().map_err(|_| {
                anyhow::anyhow!(
                    "unsupported source format \"{f}\" (expected epub, pdf, mobi, or azw3)"
                )
            })?;
            associated
                .iter()
                .find(|file| file.format == fmt)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no {fmt} file linked to \"{}\"", book.title))?
        }
        None => {
            if associated.len() == 1 {
                associated[0].clone()
            } else {
                let formats = associated
                    .iter()
                    .map(|f| f.format.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::bail!(
                    "\"{}\" has multiple files ({formats}). Choose one with --from <format>",
                    book.title
                );
            }
        }
    };

    if source.format == target_format {
        anyhow::bail!("source is already {target_format}; nothing to convert");
    }

    let src_path = PathBuf::from(&source.path);
    if !src_path.is_file() {
        anyhow::bail!(
            "source file is missing on disk: {}\nRe-add it with: toku file add",
            source.path
        );
    }

    // Write the output next to the source: same stem, new extension.
    let dst_path = src_path.with_extension(target_format.as_str());
    if dst_path == src_path {
        anyhow::bail!(
            "source and output paths are identical: {}",
            src_path.display()
        );
    }
    if dst_path.exists() && !force {
        anyhow::bail!(
            "output already exists: {}\nPass --force to overwrite.",
            dst_path.display()
        );
    }

    let converter = Converter::new();
    // Optional dependency: fail cleanly (non-zero) with install guidance if absent.
    converter
        .ensure_available()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    eprintln!(
        "Converting {} → {} via ebook-convert…",
        source.format, target_format
    );
    converter
        .convert(&src_path, &dst_path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if !dst_path.is_file() {
        anyhow::bail!(
            "ebook-convert reported success but produced no output at {}",
            dst_path.display()
        );
    }

    let size_bytes = std::fs::metadata(&dst_path)
        .with_context(|| format!("reading {}", dst_path.display()))?
        .len() as i64;
    let checksum = sha256_file(&dst_path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let stored = dst_path
        .canonicalize()
        .unwrap_or_else(|_| dst_path.clone())
        .to_string_lossy()
        .to_string();

    let file = EbookFile::new(book.id, stored, target_format, size_bytes, checksum)
        .with_source("calibre-convert", Some(source.path.clone()));

    match files.add_file(&file) {
        Ok(()) => {
            eprintln!(
                "✓ Converted to {} ({}) and linked to \"{}\"\n  {}",
                target_format,
                human_size(size_bytes),
                book.title,
                dst_path.display()
            );
            Ok(())
        }
        // The output already matched an existing association for this book.
        Err(toku_files::FileError::Duplicate(_)) => {
            eprintln!(
                "✓ Converted to {} at {} (already linked to \"{}\")",
                target_format,
                dst_path.display(),
                book.title
            );
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_file_organize(
    db: &Database,
    repo: &BookRepository,
    data_dir: &Path,
    book: Option<String>,
    all: bool,
    dry_run: bool,
    copy: bool,
    output_format: &OutputFormat,
) -> Result<()> {
    use toku_files::{PathTemplate, PlanAction, apply_plan, plan_organize};

    // Exactly one of <book> or --all.
    if book.is_some() && all {
        anyhow::bail!("specify either a book or --all, not both");
    }
    if book.is_none() && !all {
        anyhow::bail!("specify a book to organize, or --all for the whole library");
    }

    let config = TokuConfig::load(data_dir).context("failed to load config")?;
    let root = match &config.files.library_root {
        Some(dir) => PathBuf::from(dir),
        None => data_dir.join("library"),
    };
    let template =
        PathTemplate::parse(&config.files.organize_template).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Which books to organize.
    let book_ids: Vec<uuid::Uuid> = match &book {
        Some(query) => vec![resolve_book(repo, query)?.id],
        None => repo.list_books()?.into_iter().map(|b| b.id).collect(),
    };

    let plan =
        plan_organize(db, &book_ids, &root, &template, copy).map_err(|e| anyhow::anyhow!("{e}"))?;

    if plan.is_empty() {
        eprintln!("No associated files to organize.");
        return Ok(());
    }

    render_organize_plan(&plan, dry_run, output_format);

    let actionable = plan
        .iter()
        .filter(|p| matches!(p.action, PlanAction::Move | PlanAction::Copy))
        .count();

    if dry_run {
        eprintln!(
            "\nDry run: {} file(s) would be {}, {} skipped. Nothing changed.",
            actionable,
            if copy { "copied" } else { "moved" },
            plan.len() - actionable,
        );
        return Ok(());
    }

    if actionable == 0 {
        eprintln!("\nEverything is already organized. Nothing to do.");
        return Ok(());
    }

    let summary = apply_plan(db, &plan).map_err(|e| anyhow::anyhow!("{e}"))?;
    eprintln!(
        "\n✓ Organized library at {}: {} moved, {} copied, {} skipped.",
        root.display(),
        summary.moved,
        summary.copied,
        summary.skipped,
    );
    Ok(())
}

fn render_organize_plan(
    plan: &[toku_files::PlannedMove],
    dry_run: bool,
    output_format: &OutputFormat,
) {
    use toku_files::PlanAction;

    let action_label = |a: &PlanAction| -> String {
        match a {
            PlanAction::Move => "move".to_string(),
            PlanAction::Copy => "copy".to_string(),
            PlanAction::Skip { reason } => format!("skip ({reason})"),
        }
    };

    match output_format {
        OutputFormat::Json => {
            let items: Vec<serde_json::Value> = plan
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "book": p.book_title,
                        "format": p.format,
                        "action": action_label(&p.action),
                        "from": p.from.to_string_lossy(),
                        "to": p.to.to_string_lossy(),
                    })
                })
                .collect();
            if let Ok(s) = serde_json::to_string_pretty(&items) {
                println!("{s}");
            }
        }
        OutputFormat::Csv => {
            println!("action,book,format,from,to");
            for p in plan {
                println!(
                    "{},\"{}\",{},\"{}\",\"{}\"",
                    action_label(&p.action),
                    p.book_title.replace('"', "\"\""),
                    p.format,
                    p.from.to_string_lossy().replace('"', "\"\""),
                    p.to.to_string_lossy().replace('"', "\"\""),
                );
            }
        }
        OutputFormat::Table => {
            use tabled::{Table, Tabled};

            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Action")]
                action: String,
                #[tabled(rename = "Book")]
                book: String,
                #[tabled(rename = "Format")]
                format: String,
                #[tabled(rename = "Target")]
                to: String,
            }
            let rows: Vec<Row> = plan
                .iter()
                .map(|p| Row {
                    action: action_label(&p.action),
                    book: p.book_title.clone(),
                    format: p.format.clone(),
                    to: p.to.to_string_lossy().to_string(),
                })
                .collect();
            let heading = if dry_run {
                "Planned changes (dry run):"
            } else {
                "Planned changes:"
            };
            eprintln!("{heading}");
            println!("{}", Table::new(rows));
        }
    }
}

fn resolve_bulk_books(
    repo: &BookRepository,
    status: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<Book>> {
    let status_filter = status
        .map(|s| ReadingStatus::from_str(s).map_err(|_| anyhow::anyhow!("invalid status: {s}")))
        .transpose()?;

    let books = if let Some(tag_name) = tag {
        repo.list_books_by_tag(tag_name)?
    } else {
        repo.list_books()?
    };

    let filtered: Vec<Book> = if let Some(st) = status_filter {
        books.into_iter().filter(|b| b.status == st).collect()
    } else {
        books
    };

    Ok(filtered)
}

fn cmd_bulk(repo: &BookRepository, action: BulkAction) -> Result<()> {
    match action {
        BulkAction::Tag {
            tag,
            status,
            existing_tag,
            dry_run,
        } => {
            let books = resolve_bulk_books(repo, status.as_deref(), existing_tag.as_deref())?;

            if books.is_empty() {
                eprintln!("No books match the given filter.");
                return Ok(());
            }

            let verb = if dry_run { "Would tag" } else { "Tagging" };
            eprintln!("{verb} {} book(s) with \"{tag}\":\n", books.len());

            for book in &books {
                if dry_run {
                    eprintln!("  [dry-run] \"{}\"", book.title);
                } else {
                    repo.add_tag_to_book(&book.id, &tag)?;
                    eprintln!("  ✓ \"{}\"", book.title);
                }
            }

            eprintln!(
                "\n{} {} book(s).",
                if dry_run { "Would tag" } else { "Tagged" },
                books.len()
            );
            Ok(())
        }
        BulkAction::Status {
            new_status,
            status,
            tag,
            dry_run,
        } => {
            let target_status = ReadingStatus::from_str(&new_status)
                .map_err(|_| anyhow::anyhow!("invalid status: {new_status}"))?;

            let books = resolve_bulk_books(repo, status.as_deref(), tag.as_deref())?;

            if books.is_empty() {
                eprintln!("No books match the given filter.");
                return Ok(());
            }

            let verb = if dry_run { "Would update" } else { "Updating" };
            eprintln!(
                "{verb} {} book(s) to status \"{}\":\n",
                books.len(),
                target_status.as_str()
            );

            let mut updated = 0;
            let mut skipped = 0;
            for book in &books {
                if book.status == target_status {
                    skipped += 1;
                    continue;
                }
                if dry_run {
                    eprintln!(
                        "  [dry-run] \"{}\" ({} → {})",
                        book.title,
                        book.status.as_str(),
                        target_status.as_str()
                    );
                } else {
                    repo.update_book_status(&book.id, target_status)?;
                    eprintln!(
                        "  ✓ \"{}\" ({} → {})",
                        book.title,
                        book.status.as_str(),
                        target_status.as_str()
                    );
                }
                updated += 1;
            }

            let past = if dry_run { "Would update" } else { "Updated" };
            eprintln!(
                "\n{past} {updated} book(s), skipped {skipped} (already {}).",
                target_status.as_str()
            );
            Ok(())
        }
        BulkAction::Delete {
            status,
            tag,
            dry_run,
        } => {
            let books = resolve_bulk_books(repo, status.as_deref(), tag.as_deref())?;

            if books.is_empty() {
                eprintln!("No books match the given filter.");
                return Ok(());
            }

            if !dry_run && status.is_none() && tag.is_none() {
                anyhow::bail!(
                    "Refusing to delete all books without a filter. \
                     Use --status or --tag to narrow the scope."
                );
            }

            let verb = if dry_run { "Would delete" } else { "Deleting" };
            eprintln!("{verb} {} book(s):\n", books.len());

            for book in &books {
                if dry_run {
                    eprintln!("  [dry-run] \"{}\"", book.title);
                } else {
                    // `delete_book` emits the Book Delete sync op atomically
                    // with the write (no-op when sync isn't configured).
                    repo.delete_book(&book.id)?;
                    eprintln!("  ✗ \"{}\"", book.title);
                }
            }

            eprintln!(
                "\n{} {} book(s).",
                if dry_run { "Would delete" } else { "Deleted" },
                books.len()
            );
            Ok(())
        }
    }
}

fn cmd_work(repo: &BookRepository, action: WorkAction, output_format: &OutputFormat) -> Result<()> {
    match action {
        WorkAction::Link { book, work } => {
            let b = resolve_book(repo, &book)?;
            // Find or create the work
            let works = repo.find_works_by_title(&work)?;
            let w = if let Some(existing) = works
                .into_iter()
                .find(|w| w.title.eq_ignore_ascii_case(&work))
            {
                existing
            } else {
                let new_work = toku_core::Work::new(&work);
                repo.create_work(&new_work)?;
                eprintln!("Created work \"{}\"", work);
                new_work
            };

            // Check if the book already has a different work
            if let Some(existing_work_id) = b.work_id {
                if existing_work_id == w.id {
                    eprintln!("\"{}\" is already linked to work \"{}\"", b.title, w.title);
                    return Ok(());
                }
                anyhow::bail!(
                    "\"{}\" is already linked to a different work ({}). Unlink it first with `toku work unlink`.",
                    b.title,
                    existing_work_id
                );
            }

            repo.link_book_to_work(&b.id, &w.id)?;
            eprintln!("Linked \"{}\" → work \"{}\"", b.title, w.title);
            Ok(())
        }
        WorkAction::Unlink { book } => {
            let b = resolve_book(repo, &book)?;
            if b.work_id.is_none() {
                eprintln!("\"{}\" is not linked to any work", b.title);
                return Ok(());
            }
            repo.unlink_book_from_work(&b.id)?;
            eprintln!("Unlinked \"{}\" from its work", b.title);
            Ok(())
        }
        WorkAction::Show { work } => {
            let works = repo.find_works_by_title(&work)?;
            if works.is_empty() {
                eprintln!("No work found matching \"{work}\"");
                return Ok(());
            }
            for w in &works {
                let editions = repo.get_work_editions(&w.id)?;
                match output_format {
                    OutputFormat::Json => {
                        let json = serde_json::json!({
                            "work_id": w.id.to_string(),
                            "title": w.title,
                            "original_language": w.original_language,
                            "first_published": w.first_published,
                            "editions": editions.iter().map(|b| serde_json::json!({
                                "id": b.id.to_string(),
                                "title": b.title,
                                "format": b.format.as_str(),
                                "status": b.status.as_str(),
                            })).collect::<Vec<_>>(),
                        });
                        println!("{}", serde_json::to_string_pretty(&json)?);
                    }
                    OutputFormat::Csv => {
                        println!("work_id,work_title,book_id,book_title,format,status");
                        for b in &editions {
                            println!(
                                "{},{},{},{},{},{}",
                                w.id, w.title, b.id, b.title, b.format, b.status
                            );
                        }
                    }
                    OutputFormat::Table => {
                        println!("Work: {}", w.title);
                        println!("  ID: {}", w.id);
                        if let Some(lang) = &w.original_language {
                            println!("  Language: {lang}");
                        }
                        if let Some(pub_date) = &w.first_published {
                            println!("  First published: {pub_date}");
                        }
                        println!("  Editions ({}):", editions.len());
                        for b in &editions {
                            println!("    • {} [{}] — {}", b.title, b.format, b.status);
                        }
                    }
                }
            }
            Ok(())
        }
        WorkAction::Auto => {
            let candidates = repo.auto_group_candidates()?;
            if candidates.is_empty() {
                eprintln!("No potential work groups found among ungrouped books.");
                return Ok(());
            }
            match output_format {
                OutputFormat::Json => {
                    let json: Vec<_> = candidates
                        .iter()
                        .map(|(key, books)| {
                            serde_json::json!({
                                "group_key": key,
                                "books": books.iter().map(|b| serde_json::json!({
                                    "id": b.id.to_string(),
                                    "title": b.title,
                                    "format": b.format.as_str(),
                                })).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&json)?);
                }
                OutputFormat::Csv => {
                    println!("group_key,book_id,title,format");
                    for (key, books) in &candidates {
                        for b in books {
                            println!("{},{},{},{}", key, b.id, b.title, b.format);
                        }
                    }
                }
                OutputFormat::Table => {
                    eprintln!(
                        "Found {} potential work group(s). Use `toku work link` to group them.\n",
                        candidates.len()
                    );
                    for (key, books) in &candidates {
                        println!("Group: {key}");
                        for b in books {
                            println!("  • {} [{}] ({})", b.title, b.format, b.id);
                        }
                        println!();
                    }
                }
            }
            Ok(())
        }
    }
}

fn cmd_merge(
    repo: &BookRepository,
    keep_query: &str,
    remove_query: &str,
    output_format: &OutputFormat,
) -> Result<()> {
    let keep = resolve_book(repo, keep_query)?;
    let remove = resolve_book(repo, remove_query)?;

    if keep.id == remove.id {
        anyhow::bail!("Cannot merge a book with itself");
    }

    // Show comparison before merge
    match output_format {
        OutputFormat::Json => {
            let json = serde_json::json!({
                "action": "merge",
                "keep": { "id": keep.id.to_string(), "title": keep.title },
                "remove": { "id": remove.id.to_string(), "title": remove.title },
            });
            repo.merge_books(&keep.id, &remove.id)?;
            let result = serde_json::json!({
                "merged": json,
                "status": "ok",
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        _ => {
            eprintln!("Merging:");
            eprintln!("  Keep:   \"{}\" ({})", keep.title, keep.id);
            eprintln!("  Remove: \"{}\" ({})", remove.title, remove.id);
            repo.merge_books(&keep.id, &remove.id)?;
            eprintln!(
                "✓ Merged successfully. All data moved to \"{}\".",
                keep.title
            );
        }
    }
    Ok(())
}

fn cmd_stats(
    repo: &BookRepository,
    year: Option<i32>,
    author: Option<&str>,
    show_mood_trends: bool,
    output_format: &OutputFormat,
) -> Result<()> {
    // Gather books — optionally filtered by author
    let books = match author {
        Some(name) => repo.list_books_by_author_name(name)?,
        None => repo.list_books()?,
    };

    let book_ids: Vec<String> = books.iter().map(|b| b.id.to_string()).collect();

    // Gather sessions — scoped by author's books if filtered, then by year
    let sessions = if author.is_some() {
        match year {
            Some(y) => repo.list_sessions_for_books_in_year(&book_ids, y)?,
            None => repo.list_sessions_for_books(&book_ids)?,
        }
    } else {
        match year {
            Some(y) => repo.list_reading_sessions_in_year(y)?,
            None => repo.list_reading_sessions()?,
        }
    };

    // Currently reading details (only relevant when not author-filtered)
    let currently_reading_details = repo.get_currently_reading_details()?;
    let currently_reading_input: Vec<CurrentlyReadingInput> = currently_reading_details
        .into_iter()
        .filter(|(book, _, _)| {
            // When author-filtered, only include that author's books
            author.is_none() || book_ids.contains(&book.id.to_string())
        })
        .map(|(book, progress, authors)| {
            let author_name = authors
                .into_iter()
                .map(|(a, _)| a.name)
                .collect::<Vec<_>>()
                .join(", ");
            CurrentlyReadingInput {
                title: book.title,
                author: author_name,
                page_count: book.page_count,
                latest_progress: progress,
            }
        })
        .collect();

    // Tag counts — scoped by author's books if filtered
    let tag_counts = if author.is_some() {
        repo.list_tag_counts_for_books(&book_ids)?
    } else {
        repo.list_tag_counts()?
    };

    // Author counts — scoped by author's books if filtered
    let author_counts = if author.is_some() {
        repo.list_author_book_counts_for_books(&book_ids)?
    } else {
        repo.list_author_book_counts()?
    };

    // Activity dates — scoped by year and/or author
    let activity_dates = if author.is_some() {
        repo.list_activity_dates_for_books(&book_ids)?
    } else {
        match year {
            Some(y) => repo.list_activity_dates_in_year(y)?,
            None => repo.list_activity_dates()?,
        }
    };

    let now = chrono::Utc::now();
    let today = chrono::Local::now().date_naive();

    // Mood tag data — only gathered when --mood-trends is requested
    let mood_tag_data = if show_mood_trends {
        repo.get_mood_tags_for_books(&book_ids)?
    } else {
        HashMap::new()
    };

    let stats = compute_stats(StatsInput {
        books: &books,
        sessions: &sessions,
        currently_reading: &currently_reading_input,
        tag_counts: &tag_counts,
        author_counts: &author_counts,
        activity_dates: &activity_dates,
        now,
        today,
        mood_tag_data: &mood_tag_data,
    });

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
            println!(
                "rating_total_rated,{}",
                stats.rating_distribution.total_rated
            );
            for (i, &count) in stats.rating_distribution.counts.iter().enumerate() {
                println!("rating_{i},{count}");
            }
            println!("unique_authors,{}", stats.author_stats.unique_count);
            println!(
                "current_streak_days,{}",
                stats.reading_streaks.current_streak_days
            );
            println!(
                "longest_streak_days,{}",
                stats.reading_streaks.longest_streak_days
            );
            println!(
                "total_active_days,{}",
                stats.reading_streaks.total_active_days
            );
            println!(
                "avg_days_to_finish,{}",
                stats
                    .avg_days_to_finish
                    .map_or("-".to_string(), |d| format!("{d:.1}"))
            );
            println!(
                "reading_speed_pages_per_hour,{}",
                stats
                    .reading_speed_pages_per_hour
                    .map_or("-".to_string(), |s| format!("{s:.1}"))
            );
            if let Some(ref b) = stats.shortest_book {
                println!("shortest_book_pages,{}", b.page_count);
            }
            if let Some(ref b) = stats.longest_book {
                println!("longest_book_pages,{}", b.page_count);
            }
        }
        OutputFormat::Table => {
            let header = match (year, author) {
                (Some(y), Some(a)) => format!("📊 Reading Statistics ({y}) — {a}"),
                (Some(y), None) => format!("📊 Reading Statistics ({y})"),
                (None, Some(a)) => format!("📊 Reading Statistics (All Time) — {a}"),
                (None, None) => "📊 Reading Statistics (All Time)".to_string(),
            };
            println!("\n{header}\n");

            // --- Library overview ---
            println!("  Books read:        {:>5}", stats.books_read);
            println!("  Currently reading: {:>5}", stats.books_reading);
            println!("  Want to read:      {:>5}", stats.books_want_to_read);
            println!("  Abandoned:         {:>5}", stats.books_abandoned);
            println!();

            // --- Reading metrics ---
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

            // --- Reading speed ---
            if let Some(speed) = stats.reading_speed_pages_per_hour {
                println!(
                    "  Reading speed:     {:>5} pages/hour",
                    format!("{speed:.1}")
                );
            }

            // --- Time to finish ---
            if let Some(avg) = stats.avg_days_to_finish {
                println!("  Avg. time to finish: {avg:.1} days");
            }

            // --- Shortest / longest ---
            if let Some(ref b) = stats.shortest_book {
                println!("  Shortest book:     {} ({} pages)", b.title, b.page_count);
            }
            if let Some(ref b) = stats.longest_book {
                println!("  Longest book:      {} ({} pages)", b.title, b.page_count);
            }

            // --- Format breakdown ---
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

            // --- Rating distribution ---
            if stats.rating_distribution.total_rated > 0 {
                println!();
                println!("  Rating distribution:");
                for i in (0..=10).rev() {
                    let count = stats.rating_distribution.counts[i];
                    if count > 0 {
                        let stars = i as f64 / 2.0;
                        let bar = "█".repeat(count.min(40));
                        println!("    {stars:>4.1}★ {bar} {count}");
                    }
                }
            }

            // --- Tag distribution ---
            if !stats.tag_distribution.is_empty() {
                println!();
                println!("  Top tags:");
                for tc in stats.tag_distribution.iter().take(10) {
                    println!("    {:<20} {:>3}", tc.name, tc.count);
                }
            }

            // --- Author stats ---
            if stats.author_stats.unique_count > 0 {
                println!();
                println!("  Authors: {} unique", stats.author_stats.unique_count);
                if !stats.author_stats.top_authors.is_empty() {
                    println!("  Top authors:");
                    for ac in &stats.author_stats.top_authors {
                        println!("    {:<20} {:>3} books", ac.name, ac.count);
                    }
                }
            }

            // --- Reading streaks ---
            let streaks = &stats.reading_streaks;
            if streaks.total_active_days > 0 {
                println!();
                println!("  Reading streaks:");
                println!("    Current:  {:>3} days", streaks.current_streak_days);
                println!("    Longest:  {:>3} days", streaks.longest_streak_days);
                println!("    Active days: {}", streaks.total_active_days);
            }

            // --- Monthly breakdown ---
            if !stats.monthly_finished.is_empty() {
                println!();
                println!("  Books finished per month:");
                for mf in &stats.monthly_finished {
                    let month_name = match mf.month {
                        1 => "Jan",
                        2 => "Feb",
                        3 => "Mar",
                        4 => "Apr",
                        5 => "May",
                        6 => "Jun",
                        7 => "Jul",
                        8 => "Aug",
                        9 => "Sep",
                        10 => "Oct",
                        11 => "Nov",
                        12 => "Dec",
                        _ => "???",
                    };
                    let bar = "█".repeat(mf.count.min(40));
                    println!("    {} {:>4} {bar} {}", month_name, mf.year, mf.count);
                }
            }

            // --- Currently reading ---
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
                    let author_str = if cr.author.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", cr.author)
                    };
                    println!("    {}{author_str}{progress}", cr.title);
                }
            }

            // --- Mood trends ---
            if !stats.mood_trends.is_empty() {
                println!();
                println!("  Mood trends:");
                for trend in &stats.mood_trends {
                    let month_name = match trend.month {
                        1 => "Jan",
                        2 => "Feb",
                        3 => "Mar",
                        4 => "Apr",
                        5 => "May",
                        6 => "Jun",
                        7 => "Jul",
                        8 => "Aug",
                        9 => "Sep",
                        10 => "Oct",
                        11 => "Nov",
                        12 => "Dec",
                        _ => "???",
                    };
                    let moods: Vec<String> = trend
                        .moods
                        .iter()
                        .map(|m| format!("{} ({})", m.name, m.count))
                        .collect();
                    println!("    {} {:>4}: {}", month_name, trend.year, moods.join(", "));
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
        if i > 0 && i.is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn cmd_shelf(
    repo: &BookRepository,
    action: ShelfAction,
    output_format: &OutputFormat,
) -> Result<()> {
    match action {
        ShelfAction::Create {
            name,
            smart,
            filter,
        } => {
            if smart {
                let filter_str = filter.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("--filter is required when creating a smart shelf")
                })?;
                let parsed = SmartFilter::parse(filter_str).map_err(|e| anyhow::anyhow!("{e}"))?;
                repo.create_smart_shelf(&name, &parsed)
                    .with_context(|| format!("failed to create smart shelf '{name}'"))?;
                eprintln!("✓ Created smart shelf \"{name}\"");
                eprintln!("  Filter: {parsed}");
            } else {
                if filter.is_some() {
                    anyhow::bail!("--filter requires --smart");
                }
                repo.create_shelf(&name)
                    .with_context(|| format!("failed to create shelf '{name}'"))?;
                eprintln!("✓ Created shelf \"{name}\"");
            }
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
                        is_smart: bool,
                        #[serde(skip_serializing_if = "Option::is_none")]
                        filter: Option<String>,
                        book_count: usize,
                    }
                    let out: Vec<ShelfOut> = shelves
                        .iter()
                        .map(|s| {
                            let count = repo
                                .list_books_in_shelf(&s.name)
                                .map(|b| b.len())
                                .unwrap_or(0);
                            let filter_display = s.smart_filter.as_ref().and_then(|json| {
                                SmartFilter::from_json(json).ok().map(|f| f.to_string())
                            });
                            ShelfOut {
                                name: s.name.clone(),
                                is_smart: s.is_smart,
                                filter: filter_display,
                                book_count: count,
                            }
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Csv => {
                    println!("name,type,filter,books");
                    for s in &shelves {
                        let count = repo
                            .list_books_in_shelf(&s.name)
                            .map(|b| b.len())
                            .unwrap_or(0);
                        let kind = if s.is_smart { "smart" } else { "regular" };
                        let filter_display = s
                            .smart_filter
                            .as_ref()
                            .and_then(|json| {
                                SmartFilter::from_json(json).ok().map(|f| f.to_string())
                            })
                            .unwrap_or_default();
                        println!("{},{kind},\"{filter_display}\",{count}", s.name);
                    }
                }
                OutputFormat::Table => {
                    for s in &shelves {
                        let count = repo
                            .list_books_in_shelf(&s.name)
                            .map(|b| b.len())
                            .unwrap_or(0);
                        if s.is_smart {
                            let filter_display = s
                                .smart_filter
                                .as_ref()
                                .and_then(|json| {
                                    SmartFilter::from_json(json).ok().map(|f| f.to_string())
                                })
                                .unwrap_or_else(|| "?".to_string());
                            println!(
                                "  📋 {} [smart: {}] ({} book{})",
                                s.name,
                                filter_display,
                                count,
                                if count == 1 { "" } else { "s" }
                            );
                        } else {
                            println!(
                                "  📚 {} ({} book{})",
                                s.name,
                                count,
                                if count == 1 { "" } else { "s" }
                            );
                        }
                    }
                }
            }
            eprintln!("\n{} shelf/shelves", shelves.len());
            Ok(())
        }
        ShelfAction::Show { name } => {
            let shelf = repo
                .get_shelf_by_name(&name)?
                .ok_or_else(|| anyhow::anyhow!("shelf '{name}' not found"))?;

            let books = repo.list_books_in_shelf(&name)?;

            if shelf.is_smart {
                let filter_display = shelf
                    .smart_filter
                    .as_ref()
                    .and_then(|json| SmartFilter::from_json(json).ok().map(|f| f.to_string()))
                    .unwrap_or_else(|| "?".to_string());
                eprintln!("📋 Smart shelf: {name}");
                eprintln!("   Filter: {filter_display}");
            } else {
                eprintln!("📚 Shelf: {name}");
            }

            if books.is_empty() {
                eprintln!("   (empty)");
            } else {
                print_books(&books, repo, output_format)?;
                eprintln!("\n{} book(s)", books.len());
            }
            Ok(())
        }
        ShelfAction::Delete { name } => {
            if repo.delete_shelf(&name)? {
                eprintln!("✓ Deleted shelf \"{name}\"");
            } else {
                anyhow::bail!("shelf '{name}' not found");
            }
            Ok(())
        }
        ShelfAction::Add { shelf, book } => {
            let found = resolve_book(repo, &book)?;
            repo.add_book_to_shelf(&found.id, &shelf)
                .with_context(|| format!("failed to add '{}' to shelf '{shelf}'", found.title))?;
            eprintln!("✓ Added \"{}\" to shelf \"{shelf}\"", found.title);
            Ok(())
        }
        ShelfAction::Remove { shelf, book } => {
            let found = resolve_book(repo, &book)?;
            repo.remove_book_from_shelf(&found.id, &shelf)?;
            eprintln!("✓ Removed \"{}\" from shelf \"{shelf}\"", found.title);
            Ok(())
        }
    }
}

/// Generate a Secret Key and/or an Emergency Kit. Standalone for now — full
/// account signup/login (SRP) lands in a later change.
fn cmd_account(action: AccountAction) -> Result<()> {
    match action {
        AccountAction::SecretKey {
            action: SecretKeyAction::Generate { out },
        } => {
            let key = toku_core::SecretKey::generate()
                .map_err(|e| anyhow::anyhow!("failed to generate secret key: {e}"))?;
            let formatted = key.format();

            match out {
                Some(path) => {
                    std::fs::write(&path, format!("{formatted}\n"))
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    eprintln!("Secret Key written to {}", path.display());
                }
                None => {
                    println!("{formatted}");
                }
            }

            eprintln!();
            eprintln!("⚠  This is your account Secret Key. It is shown once.");
            eprintln!("   Store it offline (an Emergency Kit is the easiest way).");
            eprintln!("   It cannot be recovered — there is no server-side copy.");
            Ok(())
        }

        AccountAction::EmergencyKit {
            email,
            server,
            secret_key,
            kit_format,
            out,
        } => {
            // Resolve the Secret Key: validate a provided one, or generate fresh.
            let (formatted_key, generated) = match secret_key {
                Some(raw) => {
                    let parsed = toku_core::SecretKey::parse(&raw)
                        .map_err(|e| anyhow::anyhow!("invalid secret key: {e}"))?;
                    (parsed.format(), false)
                }
                None => {
                    let key = toku_core::SecretKey::generate()
                        .map_err(|e| anyhow::anyhow!("failed to generate secret key: {e}"))?;
                    (key.format(), true)
                }
            };

            // Resolve account email (prompt if missing).
            let email = match email {
                Some(e) => e,
                None => prompt_line("Account email: ")?,
            };
            let server = server.filter(|s| !s.trim().is_empty());

            let kit = toku_core::EmergencyKit::new(email, server, formatted_key.clone());

            match kit_format {
                KitFormat::Text => write_kit_bytes(out.as_deref(), kit.to_text().as_bytes())?,
                KitFormat::Html => write_kit_bytes(out.as_deref(), kit.to_html().as_bytes())?,
                KitFormat::Pdf => {
                    let path = out.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("--out <FILE> is required for PDF output")
                    })?;
                    let bytes = account::render_pdf(&kit)?;
                    std::fs::write(path, &bytes)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    eprintln!("Emergency Kit (PDF) written to {}", path.display());
                }
            }

            if generated {
                eprintln!();
                eprintln!("⚠  A new Secret Key was generated and embedded in this kit.");
                eprintln!("   Print or store the kit offline now — the key is shown once");
                eprintln!("   and cannot be recovered from the server.");
            }
            Ok(())
        }
    }
}

/// Write kit bytes to a file, or to stdout when no path is given.
fn write_kit_bytes(out: Option<&Path>, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    match out {
        Some(path) => {
            std::fs::write(path, bytes)
                .with_context(|| format!("failed to write {}", path.display()))?;
            eprintln!("Emergency Kit written to {}", path.display());
        }
        None => {
            std::io::stdout()
                .write_all(bytes)
                .context("failed to write to stdout")?;
        }
    }
    Ok(())
}

/// Read a single line of input from the terminal, trimming the trailing newline.
fn prompt_line(prompt: &str) -> Result<String> {
    use std::io::Write as _;
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("failed to read input")?;
    Ok(buf.trim().to_string())
}

/// Read a secret without echoing it, keeping it out of shell history/argv.
fn read_password_prompt(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    rpassword::read_password().context("failed to read password")
}

/// Environment variable that supplies a backup passphrase non-interactively
/// (automation escape hatch for `toku export/import backup --encrypt`).
const BACKUP_PASSPHRASE_ENV: &str = "TOKU_BACKUP_PASSPHRASE";

/// True when a sync server is configured, i.e. the user is a sync user and
/// `--encrypt` should seal with the enrolled library key rather than a
/// passphrase (preserves pre-existing behavior for sync users).
fn sync_is_configured(data_dir: &Path) -> bool {
    TokuConfig::load(data_dir)
        .ok()
        .and_then(|c| c.sync)
        .is_some()
}

/// Read a passphrase for sealing a NEW backup: env override, else prompt twice
/// with confirmation. Rejects empty/mismatched input.
fn read_backup_passphrase_new() -> Result<String> {
    if let Ok(p) = std::env::var(BACKUP_PASSPHRASE_ENV) {
        if p.is_empty() {
            anyhow::bail!("{BACKUP_PASSPHRASE_ENV} is set but empty");
        }
        return Ok(p);
    }
    let passphrase = read_password_prompt("Backup passphrase: ")?;
    if passphrase.is_empty() {
        anyhow::bail!("backup passphrase cannot be empty");
    }
    let confirm = read_password_prompt("Confirm backup passphrase: ")?;
    if passphrase != confirm {
        anyhow::bail!("passphrases do not match");
    }
    Ok(passphrase)
}

/// Read a passphrase to OPEN an existing encrypted backup: env override, else a
/// single prompt (no confirmation).
fn read_backup_passphrase_open() -> Result<String> {
    if let Ok(p) = std::env::var(BACKUP_PASSPHRASE_ENV)
        && !p.is_empty()
    {
        return Ok(p);
    }
    let passphrase = read_password_prompt("Backup passphrase: ")?;
    if passphrase.is_empty() {
        anyhow::bail!("backup passphrase cannot be empty");
    }
    Ok(passphrase)
}

/// Read a new password twice (with confirmation), rejecting empties/mismatches.
fn read_new_password() -> Result<String> {
    let password = read_password_prompt("Account password: ")?;
    if password.is_empty() {
        anyhow::bail!("password cannot be empty");
    }
    let confirm = read_password_prompt("Confirm password: ")?;
    if password != confirm {
        anyhow::bail!("passwords do not match");
    }
    Ok(password)
}

/// Read and validate a Secret Key without echoing it.
fn read_secret_key() -> Result<toku_core::SecretKey> {
    let raw = read_password_prompt("Secret Key: ")?;
    toku_core::SecretKey::parse(&raw).map_err(|e| anyhow::anyhow!("invalid Secret Key: {e}"))
}

/// Print, to stderr, what a first-opt-in backfill uploaded and — always — the
/// classes of data sync does not cover, so the user's "what reached the server"
/// mental model stays correct (ADR-013 D2: non-silent by construction).
fn report_backfill(report: &sync::orchestrator::BackfillReport) {
    if report.ops_total == 0 {
        eprintln!("Existing library: nothing new to upload (already staged or empty).");
    } else {
        eprintln!(
            "Uploaded your existing library: {} book(s), {} session(s), \
             {} progress entr{}, {} tag(s) \u{2014} {} ops ({} accepted).",
            report.books,
            report.sessions,
            report.progress,
            if report.progress == 1 { "y" } else { "ies" },
            report.tags,
            report.ops_total,
            report.pushed,
        );
    }
    eprintln!(
        "Note: sync does not cover ebook file binaries (they stay on this device), \
         nor authors, shelves, works, series, or ISBNs yet (tracked in #208)."
    );
}

/// Render an Emergency Kit, choosing the format from the output path's extension
/// (`.pdf`/`.html`, else text). With no path the plain-text kit is printed.
fn render_emergency_kit(kit: &toku_core::EmergencyKit, out: Option<&Path>) -> Result<()> {
    match out {
        None => {
            eprintln!();
            eprintln!("{}", kit.to_text());
            Ok(())
        }
        Some(path) => {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            let bytes = match ext.as_str() {
                "pdf" => account::render_pdf(kit)?,
                "html" => kit.to_html().into_bytes(),
                _ => kit.to_text().into_bytes(),
            };
            std::fs::write(path, &bytes)
                .with_context(|| format!("failed to write {}", path.display()))?;
            eprintln!("Emergency Kit written to {}", path.display());
            Ok(())
        }
    }
}

fn cmd_sync(data_dir: &Path, action: SyncAction, output_format: &OutputFormat) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    let token_store = sync::token_store::TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();

    /// Helper: get sync config or error with a helpful message.
    fn require_sync(config: &toku_core::TokuConfig) -> Result<&toku_core::SyncConfig> {
        config
            .sync
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("sync is not configured. Run `toku sync init` first."))
    }

    /// Helper: get auth token for the configured server.
    fn require_token(token_store: &sync::token_store::TokenStore, server: &str) -> Result<String> {
        token_store.load(server)?.ok_or_else(|| {
            anyhow::anyhow!("no auth token found for {server}. Run `toku sync init` first.")
        })
    }

    match action {
        SyncAction::Init {
            server,
            library_id,
            device_name,
            passphrase,
        } => {
            // Client-side E2E encryption is mandatory for hosted mode
            // (zero-knowledge, issue #121): always prompt for a passphrase.
            // The legacy `--passphrase` flag is accepted but no longer required.
            let _ = passphrase;
            eprintln!(
                "note: `toku sync init` is deprecated. Prefer `toku sync signup` (new account)"
            );
            eprintln!(
                "      or `toku sync login` / `toku sync enroll` (account + Secret Key auth)."
            );
            eprintln!(
                "Hosted sync uses zero-knowledge encryption. Choose a passphrase to protect your library."
            );
            eprint!("Encryption passphrase: ");
            let pass = rpassword::read_password().context("failed to read passphrase")?;
            if pass.is_empty() {
                anyhow::bail!("passphrase cannot be empty (encryption is mandatory)");
            }
            eprint!("Confirm passphrase: ");
            let confirm = rpassword::read_password().context("failed to read confirmation")?;
            if pass != confirm {
                anyhow::bail!("passphrases do not match");
            }
            let passphrase_value = Some(pass);

            let outcome = sync::orchestrator::init(
                data_dir,
                &server,
                library_id,
                device_name,
                passphrase_value.as_deref(),
            )?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "device_id": outcome.device_id,
                            "library_id": outcome.library_id,
                            "device_name": outcome.device_name,
                            "server": outcome.server,
                            "encryption": outcome.encryption,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("device_id,library_id,device_name,server,encryption");
                    println!(
                        "{},{},{},{},{}",
                        outcome.device_id,
                        outcome.library_id,
                        outcome.device_name,
                        outcome.server,
                        outcome.encryption
                    );
                }
                OutputFormat::Table => {
                    eprintln!("Sync initialized");
                    eprintln!("  Server:     {}", outcome.server);
                    eprintln!(
                        "  Device:     {} ({})",
                        outcome.device_name, outcome.device_id
                    );
                    eprintln!("  Library:    {}", outcome.library_id);
                    eprintln!(
                        "  Encryption: {}",
                        if outcome.encryption {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    );
                }
            }
            Ok(())
        }

        SyncAction::Signup {
            server,
            email,
            device_name,
            kit_out,
        } => {
            let email = match email {
                Some(e) => e,
                None => prompt_line("Account email: ")?,
            };

            eprintln!("Choose an account password. It is never sent to the server.");
            let password = read_new_password()?;

            // Generate the high-entropy Secret Key on this device.
            let secret_key = toku_core::SecretKey::generate()
                .map_err(|e| anyhow::anyhow!("failed to generate Secret Key: {e}"))?;

            let outcome = sync::orchestrator::signup(
                data_dir,
                &server,
                &email,
                &password,
                &secret_key,
                device_name,
            )?;

            // Render the Emergency Kit exactly once.
            let kit = toku_core::EmergencyKit::new(
                outcome.email.clone(),
                Some(outcome.server.clone()),
                outcome.secret_key.clone(),
            );
            render_emergency_kit(&kit, kit_out.as_deref())?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "user_id": outcome.user_id,
                            "email": outcome.email,
                            "role": outcome.role,
                            "device_id": outcome.device_id,
                            "library_id": outcome.library_id,
                            "device_name": outcome.device_name,
                            "server": outcome.server,
                            "device_status": outcome.device_status,
                            "secret_key": outcome.secret_key,
                            "backfill": {
                                "books": outcome.backfill.books,
                                "sessions": outcome.backfill.sessions,
                                "progress": outcome.backfill.progress,
                                "tags": outcome.backfill.tags,
                                "ops_total": outcome.backfill.ops_total,
                                "pushed": outcome.backfill.pushed,
                            },
                        }))?
                    );
                }
                _ => {
                    eprintln!();
                    eprintln!("Account created on {}", outcome.server);
                    eprintln!("  Email:   {}", outcome.email);
                    eprintln!("  Role:    {}", outcome.role);
                    eprintln!(
                        "  Device:  {} ({}, {})",
                        outcome.device_name, outcome.device_id, outcome.device_status
                    );
                    eprintln!("  Library: {}", outcome.library_id);
                    eprintln!();
                    report_backfill(&outcome.backfill);
                    eprintln!();
                    eprintln!("⚠  Your Secret Key is shown only once:");
                    eprintln!("     {}", outcome.secret_key);
                    eprintln!("   Store the Emergency Kit offline. It cannot be recovered.");
                }
            }
            Ok(())
        }

        SyncAction::Login { server, email } => {
            let email = match email {
                Some(e) => e,
                None => prompt_line("Account email: ")?,
            };
            let password = read_password_prompt("Account password: ")?;
            let secret_key = read_secret_key()?;

            let outcome =
                sync::orchestrator::login(data_dir, &server, &email, &password, &secret_key)?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "user_id": outcome.user_id,
                            "email": outcome.email,
                            "role": outcome.role,
                            "server": outcome.server,
                            "data_key_unlocked": outcome.data_key_unlocked,
                            "bootstrap": outcome.bootstrap.as_ref().map(|b| serde_json::json!({
                                "snapshot_applied": b.snapshot_applied,
                                "snapshot_books": b.snapshot_books,
                                "pulled": b.pulled,
                                "applied": b.applied,
                            })),
                        }))?
                    );
                }
                _ => {
                    eprintln!("Logged in to {} as {}", outcome.server, outcome.email);
                    if !outcome.data_key_unlocked {
                        eprintln!(
                            "note: the encryption key was not unlocked (the server's account-keys \
                             endpoint is unavailable). Sync of encrypted data may not work yet."
                        );
                    }
                    if let Some(bootstrap) = &outcome.bootstrap {
                        if bootstrap.snapshot_applied {
                            eprintln!(
                                "Restored your library: applied a server snapshot ({} books), \
                                 then pulled {} more ops.",
                                bootstrap.snapshot_books, bootstrap.pulled
                            );
                        } else {
                            eprintln!("Restored your library: pulled {} ops.", bootstrap.pulled);
                        }
                    }
                }
            }
            Ok(())
        }

        SyncAction::Enroll {
            server,
            email,
            library_id,
            device_name,
        } => {
            let email = match email {
                Some(e) => e,
                None => prompt_line("Account email: ")?,
            };
            let password = read_password_prompt("Account password: ")?;
            let secret_key = read_secret_key()?;

            let outcome = sync::orchestrator::enroll(
                data_dir,
                &server,
                &email,
                &password,
                &secret_key,
                device_name,
                library_id,
            )?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "user_id": outcome.user_id,
                            "email": outcome.email,
                            "device_id": outcome.device_id,
                            "library_id": outcome.library_id,
                            "device_name": outcome.device_name,
                            "server": outcome.server,
                            "device_status": outcome.device_status,
                            "backfill": outcome.backfill.as_ref().map(|b| serde_json::json!({
                                "books": b.books,
                                "sessions": b.sessions,
                                "progress": b.progress,
                                "tags": b.tags,
                                "ops_total": b.ops_total,
                                "pushed": b.pushed,
                            })),
                            "bootstrap": outcome.bootstrap.as_ref().map(|b| serde_json::json!({
                                "snapshot_applied": b.snapshot_applied,
                                "snapshot_books": b.snapshot_books,
                                "pulled": b.pulled,
                                "applied": b.applied,
                            })),
                        }))?
                    );
                }
                _ => {
                    eprintln!(
                        "Enrolled device {} ({}) into library {}",
                        outcome.device_name, outcome.device_id, outcome.library_id
                    );
                    if outcome.device_status == "pending" {
                        eprintln!(
                            "This device is pending approval by an existing trusted device. \
                             Once approved, run `toku sync login` to activate it \u{2014} it will \
                             restore your library then."
                        );
                    }
                    if let Some(backfill) = &outcome.backfill {
                        report_backfill(backfill);
                    }
                    if let Some(bootstrap) = &outcome.bootstrap {
                        if bootstrap.snapshot_applied {
                            eprintln!(
                                "Restored your library: applied a server snapshot ({} books), \
                                 then pulled {} more ops.",
                                bootstrap.snapshot_books, bootstrap.pulled
                            );
                        } else {
                            eprintln!("Restored your library: pulled {} ops.", bootstrap.pulled);
                        }
                    }
                }
            }
            Ok(())
        }

        SyncAction::Status => {
            let status = sync::orchestrator::status(data_dir)?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "enabled": status.enabled,
                            "server": status.server,
                            "device_id": status.device_id,
                            "device_name": status.device_name,
                            "library_id": status.library_id,
                            "encryption": status.encryption,
                            "pending_ops": status.pending_ops,
                            "push_cursor": status.push_cursor,
                            "pull_cursor": status.pull_cursor,
                            "device_count": status.device_count,
                            "unresolved_conflicts": status.unresolved_conflicts,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("key,value");
                    println!("enabled,{}", status.enabled);
                    println!("server,{}", status.server);
                    println!("device,{}", status.device_name);
                    println!("pending_ops,{}", status.pending_ops);
                    println!("device_count,{}", status.device_count);
                    println!("unresolved_conflicts,{}", status.unresolved_conflicts);
                }
                OutputFormat::Table => {
                    eprintln!("Sync: enabled");
                    eprintln!("Server: {}", status.server);
                    eprintln!("Device: {} ({})", status.device_name, status.device_id);
                    eprintln!("Library: {}", status.library_id);
                    eprintln!("Pending ops: {}", status.pending_ops);
                    eprintln!(
                        "Last push cursor: {}",
                        status.push_cursor.as_deref().unwrap_or("none")
                    );
                    eprintln!(
                        "Last pull cursor: {}",
                        status.pull_cursor.as_deref().unwrap_or("none")
                    );
                    eprintln!(
                        "Encryption: {}",
                        if status.encryption {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    );
                    if status.device_count > 0 {
                        eprintln!("Devices: {} registered", status.device_count);
                    }
                    if status.unresolved_conflicts > 0 {
                        eprintln!(
                            "Conflicts: {} unresolved (run `toku sync conflicts`)",
                            status.unresolved_conflicts
                        );
                    } else {
                        eprintln!("Conflicts: none");
                    }
                }
            }
            Ok(())
        }

        SyncAction::Push => {
            let outcome = sync::orchestrator::push(data_dir)?;

            if outcome.up_to_date {
                match output_format {
                    OutputFormat::Json => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "pushed": 0,
                                "status": "up_to_date",
                            }))?
                        );
                    }
                    _ => eprintln!("Nothing to push already up to date"),
                }
                return Ok(());
            }

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "pushed": outcome.pushed,
                            "accepted": outcome.accepted,
                            "duplicates": outcome.duplicates,
                            "cursor": outcome.cursor,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("pushed,accepted,duplicates");
                    println!(
                        "{},{},{}",
                        outcome.pushed, outcome.accepted, outcome.duplicates
                    );
                }
                OutputFormat::Table => {
                    eprintln!(
                        "Pushed {} ops ({} duplicates)",
                        outcome.accepted, outcome.duplicates
                    );
                }
            }
            Ok(())
        }

        SyncAction::Pull => {
            let outcome = sync::orchestrator::pull(data_dir)?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "pulled": outcome.pulled,
                            "cursor": outcome.cursor,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("pulled,cursor");
                    println!(
                        "{},{}",
                        outcome.pulled,
                        outcome.cursor.as_deref().unwrap_or("")
                    );
                }
                OutputFormat::Table => {
                    if outcome.pulled == 0 {
                        eprintln!("Nothing to pull already up to date");
                    } else {
                        eprintln!("Pulled {} ops", outcome.pulled);
                    }
                }
            }
            Ok(())
        }

        SyncAction::Bootstrap { reset_cursor } => {
            let outcome = sync::orchestrator::bootstrap(data_dir, reset_cursor)?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "snapshot_applied": outcome.snapshot_applied,
                            "snapshot_books": outcome.snapshot_books,
                            "pulled": outcome.pulled,
                            "applied": outcome.applied,
                            "reset_cursor": reset_cursor,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("snapshot_applied,snapshot_books,pulled,applied");
                    println!(
                        "{},{},{},{}",
                        outcome.snapshot_applied,
                        outcome.snapshot_books,
                        outcome.pulled,
                        outcome.applied
                    );
                }
                OutputFormat::Table => {
                    if reset_cursor {
                        eprintln!("Reset pull cursor; re-syncing from scratch.");
                    }
                    if outcome.snapshot_applied {
                        eprintln!(
                            "Applied server snapshot ({} books), then pulled {} ops.",
                            outcome.snapshot_books, outcome.pulled
                        );
                    } else if outcome.pulled == 0 {
                        eprintln!("Already up to date nothing to restore.");
                    } else {
                        eprintln!("No snapshot on server; pulled {} ops.", outcome.pulled);
                    }
                }
            }
            Ok(())
        }

        SyncAction::Devices => {
            // Prefer the authenticated, user-scoped listing when logged in;
            // fall back to the library-scoped device list otherwise.
            let user_session = config.sync.as_ref().and_then(|sc| {
                token_store
                    .load_user_session(&sc.server)
                    .ok()
                    .flatten()
                    .map(|t| (sc.server.clone(), t))
            });

            if let Some((server, _)) = user_session {
                let devices = sync::orchestrator::account_devices(data_dir, &server)?;
                match output_format {
                    OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&devices)?),
                    OutputFormat::Csv => {
                        println!("device_id,device_name,status,library_id,last_seen,created_at");
                        for d in &devices {
                            println!(
                                "{},{},{},{},{},{}",
                                d.device_id,
                                d.device_name,
                                d.status,
                                d.library_id,
                                d.last_seen.as_deref().unwrap_or(""),
                                d.created_at
                            );
                        }
                    }
                    OutputFormat::Table => {
                        if devices.is_empty() {
                            eprintln!("No devices registered.");
                            return Ok(());
                        }
                        use tabled::{Table, Tabled};
                        #[derive(Tabled)]
                        struct Row {
                            #[tabled(rename = "Device ID")]
                            id: String,
                            #[tabled(rename = "Name")]
                            name: String,
                            #[tabled(rename = "Status")]
                            status: String,
                            #[tabled(rename = "Last Seen")]
                            last_seen: String,
                            #[tabled(rename = "Registered")]
                            created: String,
                        }
                        let rows: Vec<Row> = devices
                            .iter()
                            .map(|d| Row {
                                id: d.device_id.clone(),
                                name: d.device_name.clone(),
                                status: d.status.clone(),
                                last_seen: d.last_seen.clone().unwrap_or_else(|| "n/a".into()),
                                created: d.created_at.clone(),
                            })
                            .collect();
                        println!("{}", Table::new(rows));
                    }
                }
                return Ok(());
            }

            let devices = sync::orchestrator::devices(data_dir)?;

            match output_format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&devices)?),
                OutputFormat::Csv => {
                    println!("device_id,device_name,last_seen,created_at");
                    for d in &devices {
                        println!(
                            "{},{},{},{}",
                            d.device_id,
                            d.device_name,
                            d.last_seen.as_deref().unwrap_or(""),
                            d.created_at
                        );
                    }
                }
                OutputFormat::Table => {
                    if devices.is_empty() {
                        eprintln!("No devices registered.");
                        return Ok(());
                    }
                    use tabled::{Table, Tabled};
                    #[derive(Tabled)]
                    struct Row {
                        #[tabled(rename = "Device ID")]
                        id: String,
                        #[tabled(rename = "Name")]
                        name: String,
                        #[tabled(rename = "Last Seen")]
                        last_seen: String,
                        #[tabled(rename = "Registered")]
                        created: String,
                    }
                    let rows: Vec<Row> = devices
                        .iter()
                        .map(|d| Row {
                            id: d.device_id.clone(),
                            name: d.device_name.clone(),
                            last_seen: d.last_seen.clone().unwrap_or_else(|| "n/a".into()),
                            created: d.created_at.clone(),
                        })
                        .collect();
                    println!("{}", Table::new(rows));
                }
            }
            Ok(())
        }

        SyncAction::Deregister { device_id } => {
            let sync_config = require_sync(&config)?;
            let server = &sync_config.server;
            let token = require_token(&token_store, server)?;

            let client = sync::client::SyncClient::new(server)?;
            rt.block_on(client.deregister_device(&token, &device_id))?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "deleted": true,
                            "device_id": device_id,
                        }))?
                    );
                }
                _ => eprintln!("Deregistered device {device_id}"),
            }
            Ok(())
        }

        SyncAction::Disable => {
            if config.sync.is_none() {
                eprintln!("Sync is not configured.");
                return Ok(());
            }
            let sync_config = config.sync.as_ref().unwrap();
            let server = &sync_config.server;
            let _ = token_store.delete(server);
            let _ = token_store.delete_sync_key(server);
            let _ = token_store.delete_user_session(server);

            let mut config = config;
            config.sync = None;
            config
                .save(data_dir)
                .map_err(|e| anyhow::anyhow!("failed to save config: {e}"))?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({ "sync": "disabled" }))?
                    );
                }
                _ => {
                    eprintln!("Sync disabled. Local data preserved.");
                    eprintln!("  Run `toku sync init` to re-enable.");
                }
            }
            Ok(())
        }

        SyncAction::Purge { days } => {
            let db_path = data_dir.join("toku.db");
            let db = Database::open(&db_path).context("failed to open database")?;
            let repo = toku_db::BookRepository::new(&db);
            let purged = repo.purge_tombstones(days)?;
            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "purged": purged,
                            "retention_days": days,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("purged,retention_days");
                    println!("{purged},{days}");
                }
                OutputFormat::Table => {
                    if purged == 0 {
                        eprintln!("No tombstones older than {days} days to purge");
                    } else {
                        eprintln!("Purged {purged} tombstoned book(s) older than {days} days");
                    }
                }
            }
            Ok(())
        }

        SyncAction::Rekey => {
            let sync_config = require_sync(&config)?;
            let server = &sync_config.server;
            let token = require_token(&token_store, server)?;
            let client = sync::client::SyncClient::new(server)?;

            eprint!("Old passphrase: ");
            let old_passphrase =
                rpassword::read_password().context("failed to read old passphrase")?;
            if old_passphrase.is_empty() {
                anyhow::bail!("old passphrase cannot be empty");
            }

            eprint!("New passphrase: ");
            let new_passphrase =
                rpassword::read_password().context("failed to read new passphrase")?;
            if new_passphrase.is_empty() {
                anyhow::bail!("new passphrase cannot be empty");
            }
            eprint!("Confirm new passphrase: ");
            let confirm =
                rpassword::read_password().context("failed to read passphrase confirmation")?;
            if new_passphrase != confirm {
                anyhow::bail!("passphrases do not match");
            }

            rt.block_on(async {
                eprintln!("Fetching library salt...");
                let salt_result = client.get_salt(&token).await?;
                let old_salt_b64 = salt_result
                    .salt
                    .ok_or_else(|| anyhow::anyhow!("library has no salt"))?;
                let old_salt_bytes = base64::engine::general_purpose::STANDARD
                    .decode(&old_salt_b64)
                    .context("invalid salt encoding")?;
                let old_salt: [u8; 16] = old_salt_bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid salt length"))?;
                let old_key = toku_core::SyncKey::derive(&old_passphrase, &old_salt)
                    .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;

                eprintln!("Pulling all ops from server...");
                let pull_result = client.pull_all_ops(&token).await?;
                let total = pull_result.ops.len();
                eprintln!("  {} ops to re-encrypt", total);
                if total == 0 {
                    anyhow::bail!("no ops on server");
                }

                let new_salt = toku_core::SyncKey::generate_salt()?;
                let new_key = toku_core::SyncKey::derive(&new_passphrase, &new_salt)
                    .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
                let new_salt_b64 = base64::engine::general_purpose::STANDARD.encode(new_salt);

                eprintln!("Re-encrypting ops...");
                let mut re_encrypted_ops = Vec::with_capacity(total);
                for (i, wire_op) in pull_result.ops.iter().enumerate() {
                    let mut re_wire = wire_op.clone();
                    if wire_op.payload.is_object() && wire_op.payload.get("ev").is_some() {
                        let envelope: toku_core::EncryptedEnvelope =
                            serde_json::from_value(wire_op.payload.clone())
                                .context("invalid encrypted envelope")?;
                        let entity_type: toku_core::EntityType = wire_op
                            .entity_type
                            .parse()
                            .map_err(|_| anyhow::anyhow!("invalid entity_type"))?;
                        let entity_id: uuid::Uuid = wire_op
                            .entity_id
                            .parse()
                            .map_err(|e| anyhow::anyhow!("invalid entity_id: {e}"))?;
                        let op_type: toku_core::OpType = wire_op
                            .op_type
                            .parse()
                            .map_err(|_| anyhow::anyhow!("invalid op_type"))?;
                        let plaintext = toku_core::decrypt_fields(
                            &old_key,
                            &envelope,
                            &entity_type,
                            &entity_id,
                            &op_type,
                        )
                        .map_err(|e| {
                            anyhow::anyhow!("decryption failed for op {}: {e}", wire_op.op_id)
                        })?;
                        let new_envelope = toku_core::encrypt_fields(
                            &new_key,
                            &plaintext,
                            &entity_type,
                            &entity_id,
                            &op_type,
                        )
                        .map_err(|e| anyhow::anyhow!("re-encryption failed: {e}"))?;
                        re_wire.payload = serde_json::to_value(&new_envelope)?;
                    } else if !wire_op.payload.is_null() {
                        // Zero-knowledge invariant: every op payload is either an
                        // encrypted envelope or null. A plaintext payload means the
                        // server is holding readable content — refuse to proceed.
                        anyhow::bail!(
                            "op {} has a plaintext payload; refusing to rekey readable data",
                            wire_op.op_id
                        );
                    }
                    re_encrypted_ops.push(re_wire);
                    if (i + 1) % 100 == 0 || i + 1 == total {
                        eprint!("\r  {}/{} ops re-encrypted", i + 1, total);
                    }
                }
                eprintln!();

                eprintln!("Uploading re-encrypted ops...");
                let rekey_result = client
                    .rekey(&token, &new_salt_b64, &re_encrypted_ops)
                    .await
                    .context("rekey request failed")?;

                // Re-encrypt the server snapshot (if any) under the new key so a
                // later bootstrap can still decrypt it. The snapshot keeps its
                // original HLC, so re-uploading prunes nothing extra.
                if let Some(snap) = client.download_snapshot(&token).await? {
                    let old_envelope: toku_core::EncryptedEnvelope =
                        serde_json::from_str(&snap.snapshot_json)
                            .context("stored snapshot is not an encrypted envelope")?;
                    let snapshot_json = toku_core::decrypt_snapshot(&old_key, &old_envelope)
                        .map_err(|e| anyhow::anyhow!("failed to decrypt snapshot: {e}"))?;
                    let new_envelope = toku_core::encrypt_snapshot(&new_key, &snapshot_json)
                        .map_err(|e| anyhow::anyhow!("failed to re-encrypt snapshot: {e}"))?;
                    let blob = serde_json::to_string(&new_envelope)
                        .context("failed to serialize re-encrypted snapshot")?;
                    client
                        .upload_snapshot(&token, &blob, &snap.hlc_at_snapshot)
                        .await
                        .context("failed to re-upload re-encrypted snapshot")?;
                    eprintln!("Re-encrypted server snapshot under new key");
                }

                token_store.store_sync_key(server, new_key.as_exported_bytes())?;
                eprintln!(
                    "Re-keyed {} ops with new passphrase",
                    rekey_result.ops_replaced
                );
                Ok(())
            })
        }

        SyncAction::Compact => {
            let sync_config = require_sync(&config)?;
            let server = &sync_config.server;
            let token = require_token(&token_store, server)?;

            let db_path = data_dir.join("toku.db");
            let db = Database::open(&db_path).context("failed to open database")?;
            let sync_repo = toku_db::SyncRepository::new(&db);
            let snapshot_repo = toku_db::SnapshotRepository::new(&db);
            let client = sync::client::SyncClient::new(server)?;

            let device = sync_repo.get_device()?.ok_or_else(|| {
                anyhow::anyhow!("no device identity found. Run `toku sync init` first.")
            })?;
            let mut clock = toku_core::HybridClock::new(&device.device_id);
            let hlc_str = clock.now().to_canonical();

            // Zero-knowledge: snapshots are encrypted client-side before upload
            // so the server only ever stores ciphertext (issue #121).
            let key_bytes = token_store.load_sync_key(server)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "hosted sync requires client-side encryption but no key is configured.\n\
                     Run `toku sync init --passphrase` to enroll this device with encryption."
                )
            })?;
            let key = toku_core::SyncKey::from_exported_bytes(&key_bytes)
                .map_err(|e| anyhow::anyhow!("stored sync key is invalid: {e}"))?;

            eprintln!("Creating snapshot...");
            let snapshot = snapshot_repo
                .export_snapshot(device.device_id, &hlc_str)
                .context("failed to export snapshot")?;
            let snapshot_json =
                serde_json::to_string(&snapshot).context("failed to serialize snapshot")?;
            let size_kb = snapshot_json.len() / 1024;
            eprintln!(
                "  {} books, {} sessions, {} tags ({} KB)",
                snapshot.library.books.len(),
                snapshot.library.sessions.len(),
                snapshot.library.tags.len(),
                size_kb
            );

            // Encrypt the snapshot and upload only the ciphertext envelope.
            let envelope = toku_core::encrypt_snapshot(&key, &snapshot_json)
                .map_err(|e| anyhow::anyhow!("failed to encrypt snapshot: {e}"))?;
            let encrypted_blob = serde_json::to_string(&envelope)
                .context("failed to serialize encrypted snapshot")?;

            eprintln!("Uploading encrypted snapshot and pruning old ops...");
            let result = rt.block_on(async {
                client
                    .upload_snapshot(&token, &encrypted_blob, &hlc_str)
                    .await
            })?;

            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "ops_pruned": result.ops_pruned,
                            "snapshot_size_bytes": snapshot_json.len(),
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("ops_pruned,snapshot_size_bytes");
                    println!("{},{}", result.ops_pruned, snapshot_json.len());
                }
                OutputFormat::Table => {
                    eprintln!("Snapshot uploaded, {} ops pruned", result.ops_pruned);
                }
            }
            Ok(())
        }

        SyncAction::Migrate { email, kit_out } => {
            let sync_config = require_sync(&config)?;
            let server = sync_config.server.clone();
            eprintln!(
                "Migrating this device from the legacy relay model to an account on {server}."
            );
            eprintln!(
                "A new Secret Key and account password protect all server data going forward;"
            );
            eprintln!("legacy single-passphrase access is closed once this completes.");

            let email = match email {
                Some(e) => e,
                None => prompt_line("Account email: ")?,
            };
            eprintln!("Choose an account password. It is never sent to the server.");
            let password = read_new_password()?;
            let secret_key = toku_core::SecretKey::generate()
                .map_err(|e| anyhow::anyhow!("failed to generate Secret Key: {e}"))?;

            eprintln!("Re-protecting server data under the new key hierarchy...");
            let outcome = sync::orchestrator::migrate(data_dir, &email, &password, &secret_key)?;

            let kit = toku_core::EmergencyKit::new(
                outcome.email.clone(),
                Some(outcome.server.clone()),
                outcome.secret_key.clone(),
            );
            render_emergency_kit(&kit, kit_out.as_deref())?;

            let _ = &token_store;
            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "user_id": outcome.user_id,
                            "email": outcome.email,
                            "role": outcome.role,
                            "server": outcome.server,
                            "library_id": outcome.library_id,
                            "device_id": outcome.device_id,
                            "adopted_libraries": outcome.adopted_libraries,
                            "adopted_devices": outcome.adopted_devices,
                            "ops_reencrypted": outcome.ops_reencrypted,
                            "ops_replaced": outcome.ops_replaced,
                            "had_encryption": outcome.had_encryption,
                            "secret_key": outcome.secret_key,
                        }))?
                    );
                }
                _ => {
                    eprintln!();
                    eprintln!("Migration complete on {}", outcome.server);
                    eprintln!("  Email:    {} ({})", outcome.email, outcome.role);
                    eprintln!("  Library:  {}", outcome.library_id);
                    eprintln!(
                        "  Adopted:  {} libraries, {} devices",
                        outcome.adopted_libraries, outcome.adopted_devices
                    );
                    eprintln!(
                        "  Re-keyed: {} ops ({})",
                        outcome.ops_replaced,
                        if outcome.had_encryption {
                            "from single passphrase"
                        } else {
                            "from plaintext"
                        }
                    );
                    eprintln!();
                    eprintln!("⚠  Your Secret Key is shown only once:");
                    eprintln!("     {}", outcome.secret_key);
                    eprintln!("   Store the Emergency Kit offline. It cannot be recovered.");
                    eprintln!("   Other devices must run `toku sync enroll` to rejoin.");
                }
            }
            Ok(())
        }

        SyncAction::Conflicts { action } => {
            let action = action.unwrap_or(ConflictAction::List);
            cmd_sync_conflicts(data_dir, action, output_format)
        }
    }
}

fn conflict_to_json(c: &toku_db::SyncConflict) -> serde_json::Value {
    serde_json::json!({
        "id": c.id,
        "entity_type": c.entity_type,
        "entity_id": c.entity_id,
        "field_name": c.field_name,
        "local_value": c.local_value,
        "remote_value": c.remote_value,
        "local_hlc": c.local_hlc,
        "remote_hlc": c.remote_hlc,
        "created_at": c.created_at,
    })
}

fn print_conflict_human(c: &toku_db::SyncConflict) {
    let field = c.field_name.as_deref().unwrap_or("");
    let local = c.local_value.as_deref().unwrap_or("(none)");
    let remote = c.remote_value.as_deref().unwrap_or("(none)");
    eprintln!("Conflict {}", c.id);
    eprintln!("  Entity: {} {} ({field})", c.entity_type, c.entity_id);
    eprintln!("  Local  [{}]: {local}", c.local_hlc);
    eprintln!("  Remote [{}]: {remote}", c.remote_hlc);
    eprintln!(
        "  Resolve: toku sync conflicts resolve {} --keep local|remote  (or --value \"...\")",
        c.id
    );
}

fn cmd_sync_conflicts(
    data_dir: &Path,
    action: ConflictAction,
    output_format: &OutputFormat,
) -> Result<()> {
    match action {
        ConflictAction::List => {
            let conflicts = sync::orchestrator::conflicts(data_dir)?;
            match output_format {
                OutputFormat::Json => {
                    let arr: Vec<_> = conflicts.iter().map(conflict_to_json).collect();
                    println!("{}", serde_json::to_string_pretty(&arr)?);
                }
                OutputFormat::Csv => {
                    println!("id,entity_type,entity_id,field_name,local_value,remote_value");
                    for c in &conflicts {
                        println!(
                            "{},{},{},{},{},{}",
                            c.id,
                            c.entity_type,
                            c.entity_id,
                            c.field_name.as_deref().unwrap_or(""),
                            c.local_value.as_deref().unwrap_or(""),
                            c.remote_value.as_deref().unwrap_or("")
                        );
                    }
                }
                OutputFormat::Table => {
                    if conflicts.is_empty() {
                        eprintln!("No unresolved conflicts.");
                    } else {
                        eprintln!("{} unresolved conflict(s):\n", conflicts.len());
                        for c in &conflicts {
                            print_conflict_human(c);
                            eprintln!();
                        }
                    }
                }
            }
            Ok(())
        }

        ConflictAction::Show { id } => {
            let conflict = sync::orchestrator::conflict(data_dir, &id)?
                .ok_or_else(|| anyhow::anyhow!("no conflict found with id {id}"))?;
            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&conflict_to_json(&conflict))?
                    );
                }
                OutputFormat::Csv => {
                    println!("id,entity_type,entity_id,field_name,local_value,remote_value");
                    println!(
                        "{},{},{},{},{},{}",
                        conflict.id,
                        conflict.entity_type,
                        conflict.entity_id,
                        conflict.field_name.as_deref().unwrap_or(""),
                        conflict.local_value.as_deref().unwrap_or(""),
                        conflict.remote_value.as_deref().unwrap_or("")
                    );
                }
                OutputFormat::Table => {
                    print_conflict_human(&conflict);
                }
            }
            Ok(())
        }

        ConflictAction::Resolve { id, keep, value } => {
            let (resolved, kept) = if let Some(value) = value {
                let resolved =
                    sync::orchestrator::resolve_conflict_with_value(data_dir, &id, Some(&value))?;
                (resolved, "custom".to_string())
            } else if let Some(keep) = keep {
                let resolved = sync::orchestrator::resolve_conflict(data_dir, &id, keep.into())?;
                let kept = match keep {
                    KeepArg::Local => "local",
                    KeepArg::Remote => "remote",
                };
                (resolved, kept.to_string())
            } else {
                anyhow::bail!("provide either --keep local|remote or --value \"...\"");
            };
            if !resolved {
                anyhow::bail!("no unresolved conflict found with id {id}");
            }
            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "resolved": id,
                            "kept": kept,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("resolved,kept");
                    println!("{id},{kept}");
                }
                OutputFormat::Table => {
                    eprintln!("Resolved conflict {id} (kept {kept}).");
                }
            }
            Ok(())
        }

        ConflictAction::ResolveAll { keep } => {
            let count = sync::orchestrator::resolve_all_conflicts(data_dir, keep.into())?;
            let kept = match keep {
                KeepArg::Local => "local",
                KeepArg::Remote => "remote",
            };
            match output_format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "resolved_count": count,
                            "kept": kept,
                        }))?
                    );
                }
                OutputFormat::Csv => {
                    println!("resolved_count,kept");
                    println!("{count},{kept}");
                }
                OutputFormat::Table => {
                    eprintln!("Resolved {count} conflict(s) (kept {kept}).");
                }
            }
            Ok(())
        }
    }
}
