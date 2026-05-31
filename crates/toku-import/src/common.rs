use std::collections::HashMap;

use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

use crate::ImportError;

/// The outcome of processing a single row.
#[derive(Debug, Clone, serde::Serialize)]
pub enum RowOutcome {
    Imported,
    Skipped,
    Updated,
    Error(String),
}

/// Progress event emitted for each row during import.
#[derive(Debug, Clone, serde::Serialize)]
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
#[derive(Debug, Clone, serde::Serialize)]
pub struct RowSummary {
    pub title: String,
    pub author: String,
    pub status: String,
}

pub const MAX_REPORT_SAMPLES: usize = 20;

/// Summary report of an import operation.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub total_rows: usize,
    pub imported: usize,
    pub skipped: usize,
    pub updated: usize,
    pub errors: usize,
    pub error_details: Vec<String>,
    pub import_id: Option<String>,
    /// Bounded sample of imported books (capped at 20).
    pub imported_samples: Vec<RowSummary>,
    /// Bounded sample of skipped (duplicate) books (capped at 20).
    pub skipped_samples: Vec<RowSummary>,
    /// Bounded sample of books updated with new tags (capped at 20).
    pub updated_samples: Vec<RowSummary>,
    /// Counts of imported books by reading status.
    pub status_counts: HashMap<String, usize>,
    /// Warnings about data that could not be imported (e.g. reviews skipped).
    pub warnings: Vec<String>,
}

impl std::fmt::Display for ImportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Import summary:")?;
        writeln!(f, "  Total rows:  {}", self.total_rows)?;
        writeln!(f, "  Imported:    {}", self.imported)?;
        writeln!(
            f,
            "  Updated:     {} (tags added to existing books)",
            self.updated
        )?;
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
        if !self.warnings.is_empty() {
            writeln!(f, "  Warnings:    {}", self.warnings.len())?;
            for w in self.warnings.iter().take(5) {
                writeln!(f, "    • {w}")?;
            }
            if self.warnings.len() > 5 {
                writeln!(f, "    ... and {} more", self.warnings.len() - 5)?;
            }
        }
        Ok(())
    }
}

/// Emit a progress event to the observer (if present).
pub fn emit_event(
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

/// Count the number of data rows in a CSV file (excludes the header).
pub fn count_csv_rows(csv_path: &std::path::Path) -> Result<usize, ImportError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(csv_path)?;
    Ok(rdr.records().count())
}

/// Record provenance for a field on a book.
pub fn set_provenance(
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
