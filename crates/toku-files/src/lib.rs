//! Ebook file association for Toku.
//!
//! Links ebook files on disk (`.epub`, `.pdf`, `.mobi`, `.azw3`) to existing
//! books, tracking format, size, and SHA-256 checksum. File binaries are
//! local-only and are never synced (see ROADMAP §6.4 / Phase 7 cut line).

use std::io::Read;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

mod convert;
mod organize;
mod repo;
mod template;
mod usage;
mod verify;

pub use convert::{ConvertError, Converter, DEFAULT_BINARY};
pub use organize::{
    OrganizeOutcome, OrganizeSummary, PlanAction, PlannedMove, apply_plan, plan_organize,
};
pub use repo::FileRepository;
pub use template::{
    PathTemplate, TemplateContext, UNKNOWN_AUTHOR, UNKNOWN_SERIES, UNKNOWN_YEAR, sanitize_segment,
};
pub use usage::{UsageTotals, usage_by_format, usage_by_format_typed, usage_by_key, usage_totals};
pub use verify::{VerifyOutcome, VerifyStatus, verify_file};

/// Supported ebook file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Epub,
    Pdf,
    Mobi,
    Azw3,
}

impl FileFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Mobi => "mobi",
            Self::Azw3 => "azw3",
        }
    }

    /// Detect a format from a path's file extension. Case-insensitive.
    pub fn from_path(path: &Path) -> Result<Self, FileError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| FileError::UnsupportedFormat("(no extension)".to_string()))?;
        ext.parse()
    }
}

impl std::fmt::Display for FileFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for FileFormat {
    type Err = FileError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "epub" => Ok(Self::Epub),
            "pdf" => Ok(Self::Pdf),
            "mobi" => Ok(Self::Mobi),
            "azw3" => Ok(Self::Azw3),
            _ => Err(FileError::UnsupportedFormat(s.to_string())),
        }
    }
}

/// Default provenance for a file record: added directly by the user.
pub const SOURCE_USER: &str = "user";

/// An ebook file associated with a book.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EbookFile {
    pub id: Uuid,
    pub book_id: Uuid,
    pub path: String,
    pub format: FileFormat,
    pub size_bytes: i64,
    /// SHA-256 checksum, hex-encoded.
    pub checksum: String,
    /// Provenance of this file association (e.g. `user`, `calibre`, `goodreads`).
    pub source: String,
    /// Optional external reference from the source (e.g. an import id or the
    /// original path in the source library). `None` for user-added files.
    pub source_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EbookFile {
    pub fn new(
        book_id: Uuid,
        path: String,
        format: FileFormat,
        size_bytes: i64,
        checksum: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            book_id,
            path,
            format,
            size_bytes,
            checksum,
            source: SOURCE_USER.to_string(),
            source_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the provenance of this file record. Use for importer-created records
    /// (e.g. `with_source("calibre", Some(original_path))`).
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>, source_ref: Option<String>) -> Self {
        self.source = source.into();
        self.source_ref = source_ref;
        self
    }
}

/// Errors raised by file association operations.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("unsupported format: {0} (expected epub, pdf, mobi, or azw3)")]
    UnsupportedFormat(String),

    #[error("duplicate file: a file with checksum {0} is already linked to this book")]
    Duplicate(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("invalid path template: {0}")]
    Template(String),

    #[error("organize failed: {0}")]
    Organize(String),

    #[error("database error: {0}")]
    Db(#[from] toku_db::DbError),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Compute the SHA-256 checksum (hex) of a file's contents.
pub fn sha256_file(path: &Path) -> Result<String, FileError> {
    let mut file = std::fs::File::open(path).map_err(|e| FileError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| FileError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_format_from_extension() {
        assert_eq!(
            FileFormat::from_path(&PathBuf::from("a.EPUB")).unwrap(),
            FileFormat::Epub
        );
        assert_eq!(
            FileFormat::from_path(&PathBuf::from("a.pdf")).unwrap(),
            FileFormat::Pdf
        );
        assert!(FileFormat::from_path(&PathBuf::from("a.txt")).is_err());
        assert!(FileFormat::from_path(&PathBuf::from("noext")).is_err());
    }

    #[test]
    fn computes_known_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.epub");
        std::fs::write(&p, b"").unwrap();
        // SHA-256 of the empty string.
        assert_eq!(
            sha256_file(&p).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
