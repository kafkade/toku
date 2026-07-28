//! Axum route handlers for the sync conflicts page.

use axum::extract::{Form, Path, State};
use axum::response::{Html, Redirect};
use toku_db::{ConflictKeep, Database, SyncRepository};

use crate::AppState;
use crate::error::WebError;
use crate::views;

#[derive(serde::Deserialize)]
pub struct ResolveForm {
    pub keep: String,
    /// Custom merged value, used only when `keep == "custom"`.
    #[serde(default)]
    pub value: Option<String>,
}

fn parse_keep(value: &str) -> Result<ConflictKeep, WebError> {
    match value {
        "local" => Ok(ConflictKeep::Local),
        "remote" => Ok(ConflictKeep::Remote),
        other => Err(WebError::Internal(format!("invalid keep value: {other}"))),
    }
}

/// `GET /conflicts` — list unresolved sync conflicts with resolution actions.
pub async fn conflicts_page(
    State(state): State<AppState>,
    csrf: crate::auth::CsrfToken,
) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();

    let conflicts = tokio::task::spawn_blocking(move || {
        let db = Database::open_no_migrate_default(&db_path)?;
        SyncRepository::new(&db).list_unresolved_conflicts()
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Html(
        views::conflicts_page(&conflicts, csrf.value()).into_string(),
    ))
}

/// `POST /conflicts/resolve/{id}` — resolve one conflict, then redirect back.
///
/// Supports keeping the local/remote side, or (`keep == "custom"`) writing an
/// arbitrary user-supplied merged value.
pub async fn resolve_conflict(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<ResolveForm>,
) -> Result<Redirect, WebError> {
    let db_path = state.db_path.clone();

    if form.keep == "custom" {
        let value = form.value.unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            let db = Database::open_default(&db_path)?;
            SyncRepository::new(&db).resolve_conflict_with_value(&id, Some(&value))
        })
        .await
        .map_err(|e| WebError::Internal(e.to_string()))??;
        return Ok(Redirect::to("/conflicts"));
    }

    let keep = parse_keep(&form.keep)?;
    tokio::task::spawn_blocking(move || {
        let db = Database::open_default(&db_path)?;
        SyncRepository::new(&db).resolve_conflict(&id, keep)
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Redirect::to("/conflicts"))
}

/// `POST /conflicts/resolve-all` — batch resolve every conflict, then redirect.
pub async fn resolve_all_conflicts(
    State(state): State<AppState>,
    Form(form): Form<ResolveForm>,
) -> Result<Redirect, WebError> {
    let keep = parse_keep(&form.keep)?;
    let db_path = state.db_path.clone();

    tokio::task::spawn_blocking(move || {
        let db = Database::open_default(&db_path)?;
        SyncRepository::new(&db).resolve_all_conflicts(keep)
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Redirect::to("/conflicts"))
}
