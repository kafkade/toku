//! Disk organization: plan and apply file moves into a managed library (issue #152).
//!
//! Organization is a two-phase operation:
//!
//! 1. [`plan_organize`] is a pure, read-only computation that resolves every
//!    associated file's target path from the configured template, handling
//!    collisions and idempotency. It never touches the filesystem or database.
//! 2. [`apply_plan`] executes a plan: it moves/copies files on disk, then updates
//!    the stored DB paths in a single transaction. Filesystem changes are rolled
//!    back if the transaction cannot commit, so DB and disk stay consistent.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::params;
use toku_db::{BookRepository, Database};
use uuid::Uuid;

use crate::template::{PathTemplate, TemplateContext};
use crate::{FileError, FileRepository};

/// What [`apply_plan`] will do with a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAction {
    /// Move the file from its current location to the target.
    Move,
    /// Copy the file, leaving the original in place.
    Copy,
    /// Leave the file untouched. `reason` explains why.
    Skip { reason: String },
}

impl PlanAction {
    fn is_actionable(&self) -> bool {
        matches!(self, PlanAction::Move | PlanAction::Copy)
    }
}

/// A single planned file operation produced by [`plan_organize`].
#[derive(Debug, Clone)]
pub struct PlannedMove {
    pub file_id: Uuid,
    pub book_id: Uuid,
    pub book_title: String,
    pub format: String,
    /// Current on-disk path.
    pub from: PathBuf,
    /// Resolved target path (equal to `from` for a skipped, already-organized file).
    pub to: PathBuf,
    pub action: PlanAction,
}

impl PlannedMove {
    fn is_actionable(&self) -> bool {
        self.action.is_actionable()
    }
}

/// Counts summarizing an applied plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrganizeSummary {
    pub moved: usize,
    pub copied: usize,
    pub skipped: usize,
}

/// Outcome of a full organize run (kept for API symmetry / future extension).
pub type OrganizeOutcome = OrganizeSummary;

/// Build an organization plan for the given books without touching disk or DB.
///
/// `book_ids` selects which books to consider; pass every book id for `--all`.
/// `root` is the managed library root; `template` drives the on-disk layout.
/// When `copy` is true, actionable files are copied instead of moved.
pub fn plan_organize(
    db: &Database,
    book_ids: &[Uuid],
    root: &Path,
    template: &PathTemplate,
    copy: bool,
) -> Result<Vec<PlannedMove>, FileError> {
    let books = BookRepository::new(db);
    let files = FileRepository::new(db);

    let mut plan = Vec::new();
    // Target paths already claimed within this plan (for collision resolution).
    let mut claimed: HashSet<PathBuf> = HashSet::new();

    for book_id in book_ids {
        let book = books.get_book(book_id)?;
        let author = books
            .get_book_authors(book_id)?
            .into_iter()
            .next()
            .map(|(a, _)| a.name)
            .unwrap_or_default();
        let series = books
            .get_book_series(book_id)?
            .into_iter()
            .next()
            .map(|(s, _)| s.name);
        let year = extract_year(book.pub_date.as_deref());

        for file in files.list_files(book_id)? {
            let ctx = TemplateContext {
                author: author.clone(),
                title: book.title.clone(),
                series: series.clone(),
                format: file.format.as_str().to_string(),
                year: year.clone(),
            };

            let from = PathBuf::from(&file.path);

            // A missing source cannot be organized.
            if !from.exists() {
                plan.push(PlannedMove {
                    file_id: file.id,
                    book_id: *book_id,
                    book_title: book.title.clone(),
                    format: file.format.as_str().to_string(),
                    from: from.clone(),
                    to: from,
                    action: PlanAction::Skip {
                        reason: "source file missing".to_string(),
                    },
                });
                continue;
            }

            let segments = template.render(&ctx)?;
            let base_target = segments
                .iter()
                .fold(root.to_path_buf(), |acc, s| acc.join(s));

            let (to, action) = resolve_target(&from, &base_target, &claimed, copy);
            claimed.insert(to.clone());

            plan.push(PlannedMove {
                file_id: file.id,
                book_id: *book_id,
                book_title: book.title.clone(),
                format: file.format.as_str().to_string(),
                from,
                to,
                action,
            });
        }
    }

    Ok(plan)
}

/// Resolve the final target for a file, handling idempotency and collisions.
fn resolve_target(
    from: &Path,
    base_target: &Path,
    claimed: &HashSet<PathBuf>,
    copy: bool,
) -> (PathBuf, PlanAction) {
    let mut candidate = base_target.to_path_buf();
    let mut n = 1;
    loop {
        // Already sitting at this exact location → nothing to do.
        if same_file(from, &candidate) {
            return (
                candidate,
                PlanAction::Skip {
                    reason: "already organized".to_string(),
                },
            );
        }

        let taken = claimed.contains(&candidate) || candidate.exists();
        if !taken {
            let action = if copy {
                PlanAction::Copy
            } else {
                PlanAction::Move
            };
            return (candidate, action);
        }

        // Collision with a different file: append a deterministic ` (n)` suffix.
        n += 1;
        candidate = with_suffix(base_target, n);
    }
}

/// Apply an organization plan: perform filesystem moves/copies, then update the
/// stored paths in a single transaction. Filesystem changes are undone if the
/// transaction cannot commit.
pub fn apply_plan(db: &Database, plan: &[PlannedMove]) -> Result<OrganizeSummary, FileError> {
    let actionable: Vec<&PlannedMove> = plan.iter().filter(|p| p.is_actionable()).collect();

    // Phase 1: filesystem. Track completed ops so we can roll them back on error.
    let mut done: Vec<&PlannedMove> = Vec::new();
    for pm in &actionable {
        if let Some(parent) = pm.to.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            undo_fs(&done);
            return Err(FileError::Organize(format!(
                "creating {}: {e}",
                parent.display()
            )));
        }
        let result = match pm.action {
            PlanAction::Move => move_file(&pm.from, &pm.to),
            PlanAction::Copy => std::fs::copy(&pm.from, &pm.to)
                .map(|_| ())
                .map_err(|e| FileError::Io(e.to_string())),
            PlanAction::Skip { .. } => Ok(()),
        };
        if let Err(e) = result {
            undo_fs(&done);
            return Err(FileError::Organize(format!(
                "{} → {}: {e}",
                pm.from.display(),
                pm.to.display()
            )));
        }
        done.push(pm);
    }

    // Phase 2: database. Update all paths atomically.
    let now = Utc::now().to_rfc3339();
    let tx = db.conn.unchecked_transaction()?;
    for pm in &actionable {
        let res = tx.execute(
            "UPDATE files SET path = ?2, updated_at = ?3 WHERE id = ?1",
            params![pm.file_id.to_string(), pm.to.to_string_lossy(), now],
        );
        if let Err(e) = res {
            // tx drops → rollback; undo the filesystem changes too.
            drop(tx);
            undo_fs(&done);
            return Err(FileError::Sqlite(e));
        }
    }
    if let Err(e) = tx.commit() {
        undo_fs(&done);
        return Err(FileError::Sqlite(e));
    }

    let mut summary = OrganizeSummary::default();
    for pm in plan {
        match pm.action {
            PlanAction::Move => summary.moved += 1,
            PlanAction::Copy => summary.copied += 1,
            PlanAction::Skip { .. } => summary.skipped += 1,
        }
    }
    Ok(summary)
}

/// Best-effort reversal of completed filesystem operations.
fn undo_fs(done: &[&PlannedMove]) {
    for pm in done.iter().rev() {
        match pm.action {
            // Move the file back to its original location.
            PlanAction::Move => {
                let _ = std::fs::rename(&pm.to, &pm.from);
            }
            // Remove the copy we created.
            PlanAction::Copy => {
                let _ = std::fs::remove_file(&pm.to);
            }
            PlanAction::Skip { .. } => {}
        }
    }
}

/// Move a file, falling back to copy+remove when `rename` fails (e.g. across
/// filesystems / mount points).
fn move_file(from: &Path, to: &Path) -> Result<(), FileError> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to).map_err(|e| FileError::Io(e.to_string()))?;
            std::fs::remove_file(from).map_err(|e| FileError::Io(e.to_string()))?;
            Ok(())
        }
    }
}

/// True when both paths resolve to the same file on disk.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Insert a ` (n)` suffix before the file extension of `path`.
fn with_suffix(path: &Path, n: usize) -> PathBuf {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let new_name = match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({}){}", &name[..dot], n, &name[dot..]),
        _ => format!("{name} ({n})"),
    };
    match path.parent() {
        Some(p) => p.join(new_name),
        None => PathBuf::from(new_name),
    }
}

/// Extract a four-digit year from an ISO-ish `pub_date` (e.g. `1969`, `1969-05`).
fn extract_year(pub_date: Option<&str>) -> Option<String> {
    let date = pub_date?.trim();
    let year: String = date.chars().take_while(|c| c.is_ascii_digit()).collect();
    if year.len() == 4 { Some(year) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_four_digit_year() {
        assert_eq!(extract_year(Some("1969")), Some("1969".to_string()));
        assert_eq!(extract_year(Some("1969-05-01")), Some("1969".to_string()));
        assert_eq!(extract_year(Some("")), None);
        assert_eq!(extract_year(Some("May 1969")), None);
        assert_eq!(extract_year(None), None);
    }

    #[test]
    fn suffix_inserts_before_extension() {
        let p = PathBuf::from("/lib/Author/Title.epub");
        assert_eq!(
            with_suffix(&p, 2),
            PathBuf::from("/lib/Author/Title (2).epub")
        );
    }

    #[test]
    fn suffix_without_extension() {
        let p = PathBuf::from("/lib/Author/Title");
        assert_eq!(with_suffix(&p, 3), PathBuf::from("/lib/Author/Title (3)"));
    }
}
