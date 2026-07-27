//! Canonical, versioned, lossless backup schema (ADR-012).
//!
//! A single set of `serde` types describes the **whole** persisted domain model.
//! [`LibraryData`] is the payload of a canonical backup (`library.json`) and is
//! also the shared shape that the sync snapshot projects its non-binary subset
//! from (ADR-012 D2).
//!
//! Every field maps to a database column. `TEXT` columns are represented as
//! `String` / `Option<String>` so a round trip preserves the stored bytes
//! exactly (ids, timestamps, HLCs, positions), and integer columns keep their
//! integer type. New optional fields and whole new entity vectors are additive
//! and default-constructed on restore (`#[serde(default)]`), so an older Toku
//! can restore a newer backup's shared subset and vice versa (ADR-012 D5).

use serde::{Deserialize, Serialize};

/// The current canonical backup / snapshot schema version.
///
/// Starts at `2`: the superseded flat `LibraryExport` was version `"1"`, and the
/// lossless format is the next generation (ADR-012 D5).
pub const BACKUP_FORMAT_VERSION: u32 = 2;

/// Metadata envelope written to `manifest.json` inside the backup ZIP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Integer schema version (see [`BACKUP_FORMAT_VERSION`]).
    pub format_version: u32,
    /// RFC 3339 timestamp of when the backup was created.
    pub created_at: String,
    /// Per-entity row counts, for a quick human/inspection summary.
    pub counts: BackupCounts,
    /// Whether `library.json` inside the ZIP is sealed (ADR-012 D4).
    #[serde(default)]
    pub encrypted: bool,
    /// The AEAD envelope when `encrypted` is true; absent for plaintext backups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<crate::crypto::EncryptedEnvelope>,
}

/// Row counts for each entity vector, surfaced in the manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupCounts {
    #[serde(default)]
    pub books: usize,
    #[serde(default)]
    pub authors: usize,
    #[serde(default)]
    pub reading_sessions: usize,
    #[serde(default)]
    pub reading_progress: usize,
    #[serde(default)]
    pub notes: usize,
    #[serde(default)]
    pub reviews: usize,
    #[serde(default)]
    pub tags: usize,
    #[serde(default)]
    pub shelves: usize,
    #[serde(default)]
    pub works: usize,
    #[serde(default)]
    pub series: usize,
    #[serde(default)]
    pub isbns: usize,
    #[serde(default)]
    pub files: usize,
    #[serde(default)]
    pub covers: usize,
}

/// The complete library payload (`library.json`).
///
/// Every persisted, user-owned table is represented. Ordering within each
/// vector is by primary key so serialization is deterministic.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryData {
    #[serde(default)]
    pub books: Vec<BookRow>,
    #[serde(default)]
    pub authors: Vec<AuthorRow>,
    #[serde(default)]
    pub book_authors: Vec<BookAuthorRow>,
    #[serde(default)]
    pub isbns: Vec<IsbnRow>,
    #[serde(default)]
    pub works: Vec<WorkRow>,
    #[serde(default)]
    pub series: Vec<SeriesRow>,
    #[serde(default)]
    pub book_series: Vec<BookSeriesRow>,
    #[serde(default)]
    pub shelves: Vec<ShelfRow>,
    #[serde(default)]
    pub book_shelves: Vec<BookShelfRow>,
    #[serde(default)]
    pub tags: Vec<TagRow>,
    #[serde(default)]
    pub book_tags: Vec<BookTagRow>,
    #[serde(default)]
    pub reading_sessions: Vec<ReadingSessionRow>,
    #[serde(default)]
    pub reading_progress: Vec<ReadingProgressRow>,
    #[serde(default)]
    pub notes: Vec<NoteRow>,
    #[serde(default)]
    pub reviews: Vec<ReviewRow>,
    #[serde(default)]
    pub user_settings: Vec<UserSettingRow>,
    #[serde(default)]
    pub metadata_provenance: Vec<ProvenanceRow>,
    #[serde(default)]
    pub entity_hlc: Vec<EntityHlcRow>,
    #[serde(default)]
    pub files: Vec<FileRow>,
    #[serde(default)]
    pub import_logs: Vec<ImportLogRow>,
    #[serde(default)]
    pub import_books: Vec<ImportBookRow>,
}

impl LibraryData {
    /// Compute per-entity counts (covers is filled in by the exporter, which
    /// knows how many cover binaries were actually written).
    pub fn counts(&self) -> BackupCounts {
        BackupCounts {
            books: self.books.len(),
            authors: self.authors.len(),
            reading_sessions: self.reading_sessions.len(),
            reading_progress: self.reading_progress.len(),
            notes: self.notes.len(),
            reviews: self.reviews.len(),
            tags: self.tags.len(),
            shelves: self.shelves.len(),
            works: self.works.len(),
            series: self.series.len(),
            isbns: self.isbns.len(),
            files: self.files.len(),
            covers: 0,
        }
    }
}

/// A row of `books` (Book = Edition), including source IDs and tombstones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookRow {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub page_count: Option<i64>,
    #[serde(default)]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    pub format: String,
    #[serde(default)]
    pub duration_minutes: Option<i64>,
    #[serde(default)]
    pub cover_hash: Option<String>,
    #[serde(default)]
    pub work_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub rating: Option<i64>,
    #[serde(default)]
    pub goodreads_id: Option<String>,
    #[serde(default)]
    pub calibre_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub deleted_by_device: Option<String>,
}

/// A row of `authors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorRow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sort_name: Option<String>,
}

/// A row of `book_authors` (role + ordering preserved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookAuthorRow {
    pub book_id: String,
    pub author_id: String,
    pub role: String,
    pub position: i64,
}

/// A row of `isbns` (all ISBN-10 / ISBN-13 per book).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsbnRow {
    pub isbn: String,
    pub book_id: String,
}

/// A row of `works`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRow {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub original_language: Option<String>,
    #[serde(default)]
    pub first_published: Option<String>,
    pub created_at: String,
}

/// A row of `series`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesRow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub total_books: Option<i64>,
}

/// A row of `book_series` (position preserved as TEXT: "1.5", "2a", ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookSeriesRow {
    pub book_id: String,
    pub series_id: String,
    #[serde(default)]
    pub position: Option<String>,
}

/// A row of `shelves` (smart-shelf definition preserved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfRow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub is_smart: bool,
    #[serde(default)]
    pub smart_filter: Option<String>,
    pub created_at: String,
}

/// A row of `book_shelves`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookShelfRow {
    pub book_id: String,
    pub shelf_id: String,
}

/// A row of `tags` (tag_type preserved — "dark" mood != "dark" genre).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    pub tag_type: String,
    pub created_at: String,
}

/// A row of `book_tags`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookTagRow {
    pub book_id: String,
    pub tag_id: String,
}

/// A row of `reading_sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingSessionRow {
    pub id: String,
    pub book_id: String,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub start_page: Option<i64>,
    #[serde(default)]
    pub end_page: Option<i64>,
    #[serde(default)]
    pub rating: Option<i64>,
    #[serde(default)]
    pub notes: Option<String>,
    pub created_at: String,
}

/// A row of `reading_progress`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingProgressRow {
    pub id: String,
    pub book_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub progress_type: String,
    pub value: i64,
    #[serde(default)]
    pub note: Option<String>,
    pub logged_at: String,
    pub created_at: String,
}

/// A row of `notes` (tombstone preserved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRow {
    pub id: String,
    pub book_id: String,
    pub content: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub deleted_by_device: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A row of `reviews` (tombstone preserved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRow {
    pub id: String,
    pub book_id: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub rating: Option<i64>,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub deleted_by_device: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A row of `user_settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSettingRow {
    pub id: String,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub sync_hlc: Option<String>,
    pub updated_at: String,
}

/// A row of `metadata_provenance` (per-field source + user-override + HLC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceRow {
    pub book_id: String,
    pub field_name: String,
    pub source: String,
    pub source_date: String,
    #[serde(default)]
    pub is_user_override: bool,
    #[serde(default)]
    pub sync_hlc: Option<String>,
}

/// A row of `sync_entity_hlc` — per-field HLC for notes/reviews/etc. LWW.
///
/// Carried so that notes/reviews merge and tombstone precedence round-trip
/// exactly (ADR-012 D3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityHlcRow {
    pub entity_type: String,
    pub entity_id: String,
    pub field_name: String,
    pub sync_hlc: String,
    #[serde(default)]
    pub device_id: Option<String>,
}

/// A row of `files` — ebook file association (binary stored separately).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRow {
    pub id: String,
    pub book_id: String,
    pub path: String,
    pub format: String,
    pub size_bytes: i64,
    /// SHA-256 checksum, hex-encoded — also the content address in the ZIP.
    pub checksum: String,
    pub source: String,
    #[serde(default)]
    pub source_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A row of `import_logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportLogRow {
    pub id: String,
    pub source: String,
    pub file_path: String,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub total_rows: i64,
    pub imported: i64,
    pub skipped: i64,
    pub errors: i64,
}

/// A row of `import_books` (which books came from which import).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportBookRow {
    pub import_id: String,
    pub book_id: String,
}
