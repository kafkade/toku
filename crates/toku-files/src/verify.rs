//! File integrity verification via SHA-256 checksums.
//!
//! Recomputes the checksum of an associated file by streaming its contents
//! (constant memory, regardless of file size) and compares it against the value
//! stored when the file was linked. Detects two failure modes:
//!
//! - **Missing** — the file no longer exists on disk at its stored path.
//! - **Mismatch** — the file exists but its contents changed (corruption, an
//!   external edit, or a truncated/partial write).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{EbookFile, FileError, sha256_file};

/// Outcome of verifying a single file against its stored checksum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyStatus {
    /// File exists and its recomputed checksum matches the stored value.
    Ok,
    /// File exists but its recomputed checksum differs from the stored value.
    Mismatch,
    /// File no longer exists on disk at its stored path.
    Missing,
}

impl VerifyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Mismatch => "mismatch",
            Self::Missing => "missing",
        }
    }

    /// Whether this status represents an integrity problem.
    pub fn is_problem(&self) -> bool {
        !matches!(self, Self::Ok)
    }
}

impl std::fmt::Display for VerifyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The result of verifying one [`EbookFile`].
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    pub file: EbookFile,
    pub status: VerifyStatus,
    /// The freshly recomputed checksum, present only when the file existed and
    /// could be read. `None` for missing files.
    pub computed: Option<String>,
}

/// Verify a single file against its stored checksum.
///
/// Streams the file's bytes to compute SHA-256 without loading it into memory.
/// Returns a [`VerifyOutcome`] describing whether the file is intact, corrupted,
/// or missing. Read errors other than "not found" surface as [`FileError`].
pub fn verify_file(file: &EbookFile) -> Result<VerifyOutcome, FileError> {
    let path = Path::new(&file.path);
    if !path.exists() {
        return Ok(VerifyOutcome {
            file: file.clone(),
            status: VerifyStatus::Missing,
            computed: None,
        });
    }
    let computed = sha256_file(path)?;
    let status = if computed == file.checksum {
        VerifyStatus::Ok
    } else {
        VerifyStatus::Mismatch
    };
    Ok(VerifyOutcome {
        file: file.clone(),
        status,
        computed: Some(computed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileFormat;
    use uuid::Uuid;

    fn file_at(path: &Path, checksum: &str) -> EbookFile {
        EbookFile::new(
            Uuid::now_v7(),
            path.to_string_lossy().to_string(),
            FileFormat::Epub,
            0,
            checksum.to_string(),
        )
    }

    // SHA-256 of the empty string.
    const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn reports_ok_when_checksum_matches() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.epub");
        std::fs::write(&p, b"").unwrap();
        let f = file_at(&p, EMPTY_SHA);
        let outcome = verify_file(&f).unwrap();
        assert_eq!(outcome.status, VerifyStatus::Ok);
        assert!(!outcome.status.is_problem());
        assert_eq!(outcome.computed.as_deref(), Some(EMPTY_SHA));
    }

    #[test]
    fn reports_mismatch_when_contents_change() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.epub");
        std::fs::write(&p, b"tampered").unwrap();
        // Stored checksum is for the empty file, so this must mismatch.
        let f = file_at(&p, EMPTY_SHA);
        let outcome = verify_file(&f).unwrap();
        assert_eq!(outcome.status, VerifyStatus::Mismatch);
        assert!(outcome.status.is_problem());
        assert_ne!(outcome.computed.as_deref(), Some(EMPTY_SHA));
    }

    #[test]
    fn reports_missing_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("gone.epub");
        let f = file_at(&p, EMPTY_SHA);
        let outcome = verify_file(&f).unwrap();
        assert_eq!(outcome.status, VerifyStatus::Missing);
        assert!(outcome.status.is_problem());
        assert!(outcome.computed.is_none());
    }
}
