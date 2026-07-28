//! Axum route handler for the read-only sync status page.
//!
//! The web dashboard is a local companion: it reads sync state straight from the
//! local config and database (no network calls), so the page stays responsive
//! even when the sync server is unreachable. Setup, push/pull, and the live
//! device list live in the CLI and native apps.

use std::path::Path;

use axum::extract::State;
use axum::response::Html;
use toku_db::{Database, SyncRepository};

use crate::AppState;
use crate::error::WebError;
use crate::views;

/// Local, read-only snapshot of sync state for the dashboard.
pub struct SyncOverview {
    pub configured: bool,
    pub server: String,
    pub device_name: String,
    pub device_id: String,
    pub library_id: String,
    pub encryption: bool,
    pub pending_ops: i64,
    pub push_cursor: Option<String>,
    pub pull_cursor: Option<String>,
    pub conflicts: i64,
}

impl SyncOverview {
    fn unconfigured() -> Self {
        SyncOverview {
            configured: false,
            server: String::new(),
            device_name: String::new(),
            device_id: String::new(),
            library_id: String::new(),
            encryption: false,
            pending_ops: 0,
            push_cursor: None,
            pull_cursor: None,
            conflicts: 0,
        }
    }
}

/// `GET /sync` — read-only sync status (config + local counters).
pub async fn sync_page(State(state): State<AppState>) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();

    let overview = tokio::task::spawn_blocking(move || -> Result<SyncOverview, WebError> {
        let data_dir = db_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| Path::new(".").to_path_buf());

        let config = toku_core::TokuConfig::load(&data_dir).unwrap_or_default();
        let Some(sync) = config.sync else {
            return Ok(SyncOverview::unconfigured());
        };

        let db = Database::open_no_migrate_default(&db_path)
            .map_err(|e| WebError::Internal(e.to_string()))?;
        let repo = SyncRepository::new(&db);

        let pending_ops = repo
            .count_unpushed_ops()
            .map_err(|e| WebError::Internal(e.to_string()))?;
        let push_cursor = repo
            .get_cursor("push_cursor")
            .map_err(|e| WebError::Internal(e.to_string()))?;
        let pull_cursor = repo
            .get_cursor("pull_cursor")
            .map_err(|e| WebError::Internal(e.to_string()))?;
        let conflicts = repo
            .count_unresolved_conflicts()
            .map_err(|e| WebError::Internal(e.to_string()))?;

        Ok(SyncOverview {
            configured: true,
            server: sync.server,
            device_name: sync.device_name,
            device_id: sync.device_id,
            library_id: sync.library_id,
            encryption: sync.encryption,
            pending_ops,
            push_cursor,
            pull_cursor,
            conflicts,
        })
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Html(views::sync_page(&overview).into_string()))
}
