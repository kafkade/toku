//! Shared sync status indicator for the dashboard header.
//!
//! The header is rendered by two separate `base()` layouts that have no access
//! to request state, so the data directory is stashed in a process-wide
//! `OnceLock` at server startup and read lazily when rendering the badge.

use std::path::PathBuf;
use std::sync::OnceLock;

use maud::{Markup, html};
use toku_db::{Database, SyncRepository};

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Record the data directory so the header badge can read sync state.
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

/// Current sync state for the header indicator.
pub struct SyncIndicator {
    pub configured: bool,
    pub conflicts: usize,
}

/// Compute the current sync indicator state (best-effort; never errors).
pub fn current() -> SyncIndicator {
    let Some(dir) = DATA_DIR.get() else {
        return SyncIndicator {
            configured: false,
            conflicts: 0,
        };
    };

    let config = toku_core::TokuConfig::load(dir).unwrap_or_default();
    if config.sync.is_none() {
        return SyncIndicator {
            configured: false,
            conflicts: 0,
        };
    }

    let conflicts = Database::open_no_migrate(&dir.join("toku.db"))
        .ok()
        .and_then(|db| SyncRepository::new(&db).count_unresolved_conflicts().ok())
        .unwrap_or(0)
        .max(0) as usize;

    SyncIndicator {
        configured: true,
        conflicts,
    }
}

/// Render the header sync badge, or nothing when sync is not configured.
pub fn header_badge() -> Markup {
    let indicator = current();
    if !indicator.configured {
        return html! {};
    }

    let (class, label) = if indicator.conflicts > 0 {
        (
            "sync-indicator sync-indicator-alert",
            format!(
                "⚠ {} conflict{}",
                indicator.conflicts,
                if indicator.conflicts == 1 { "" } else { "s" }
            ),
        )
    } else {
        ("sync-indicator sync-indicator-ok", "✓ Synced".to_string())
    };

    html! {
        a class=(class) href="/sync" title="Sync status" { (label) }
    }
}
