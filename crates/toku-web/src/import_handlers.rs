//! Axum route handlers for the import wizard.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Multipart, Path, State};
use axum::response::sse::{Event, KeepAlive};
use axum::response::{Html, Redirect, Sse};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use toku_import::{ImportEvent, ImportObserver, ImportReport, RowOutcome};

use crate::AppState;
use crate::error::WebError;
use crate::import_views;

// ── Types ───────────────────────────────────────────────────────────

/// Which import source this session is using.
#[derive(Debug, Clone)]
pub enum ImportSourceKind {
    Goodreads,
    Calibre { import_covers: bool },
    StoryGraph,
}

/// Current status of an import session.
#[derive(Debug, Clone)]
pub enum ImportSessionStatus {
    Preview,
    Running,
    Complete,
    Failed(String),
}

/// An in-progress or completed import session.
pub struct ImportSession {
    pub id: String,
    pub source: ImportSourceKind,
    pub file_path: PathBuf,
    pub status: ImportSessionStatus,
    pub dry_run_report: Option<ImportReport>,
    pub final_report: Option<ImportReport>,
    pub progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    /// Snapshot of latest progress (for SSE replay on reconnect).
    pub latest_progress: Option<ProgressEvent>,
}

/// A progress event sent over SSE.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub event_type: String, // "progress", "complete", "error"
    pub row: usize,
    pub total: usize,
    pub title: String,
    pub imported: usize,
    pub skipped: usize,
    pub updated: usize,
    pub errors: usize,
}

/// Import session storage type.
pub type ImportSessions = Arc<Mutex<HashMap<String, ImportSession>>>;

// ── Observer ────────────────────────────────────────────────────────

/// Implements `ImportObserver` by sending events through a broadcast channel.
struct WebImportObserver {
    tx: broadcast::Sender<ProgressEvent>,
    sessions: ImportSessions,
    session_id: String,
    imported: usize,
    skipped: usize,
    updated: usize,
    errors: usize,
}

impl ImportObserver for WebImportObserver {
    fn on_event(&mut self, event: &ImportEvent) -> Result<(), toku_import::ImportError> {
        match &event.outcome {
            RowOutcome::Imported => self.imported += 1,
            RowOutcome::Skipped => self.skipped += 1,
            RowOutcome::Updated => self.updated += 1,
            RowOutcome::Error(_) => self.errors += 1,
        }
        let progress = ProgressEvent {
            event_type: "progress".to_string(),
            row: event.row,
            total: event.total,
            title: event.title.clone(),
            imported: self.imported,
            skipped: self.skipped,
            updated: self.updated,
            errors: self.errors,
        };
        // Store latest progress for SSE replay
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(session) = sessions.get_mut(&self.session_id)
        {
            session.latest_progress = Some(progress.clone());
        }
        // Ignore send errors — no listeners should not abort import
        let _ = self.tx.send(progress);
        Ok(())
    }
}

// ── Handlers ────────────────────────────────────────────────────────

/// `GET /import` — import type selection page.
pub async fn import_page(csrf: crate::auth::CsrfToken) -> Html<String> {
    Html(import_views::import_page(csrf.value()).into_string())
}

/// `POST /import/upload` — handle CSV file upload (Goodreads or StoryGraph).
pub async fn upload_csv(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Redirect, WebError> {
    let mut source: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| WebError::Internal(format!("multipart error: {e}")))?
    {
        match field.name() {
            Some("source") => {
                source = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| WebError::Internal(e.to_string()))?,
                );
            }
            Some("file") => {
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| WebError::Internal(e.to_string()))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    let source_str = source.ok_or_else(|| WebError::Internal("missing source field".into()))?;
    let data = file_data.ok_or_else(|| WebError::Internal("no file uploaded".into()))?;

    if data.is_empty() {
        return Err(WebError::Internal("uploaded file is empty".into()));
    }

    let source_kind = match source_str.as_str() {
        "goodreads" => ImportSourceKind::Goodreads,
        "storygraph" => ImportSourceKind::StoryGraph,
        _ => return Err(WebError::Internal(format!("unknown source: {source_str}"))),
    };

    // Save uploaded file to temp directory
    let session_id = uuid::Uuid::now_v7().to_string();
    let temp_path = state.temp_dir.join(format!("{session_id}.csv"));
    let mut file = std::fs::File::create(&temp_path)
        .map_err(|e| WebError::Internal(format!("failed to create temp file: {e}")))?;
    file.write_all(&data)
        .map_err(|e| WebError::Internal(format!("failed to write temp file: {e}")))?;

    // Run dry-run
    let db_path = state.db_path.clone();
    let temp_path_clone = temp_path.clone();
    let source_clone = source_kind.clone();

    let report = tokio::task::spawn_blocking(move || -> Result<ImportReport, WebError> {
        let db = toku_db::Database::open_no_migrate_default(&db_path)?;
        match &source_clone {
            ImportSourceKind::Goodreads => {
                let opts = toku_import::GoodreadsImportOptions { dry_run: true };
                Ok(toku_import::import_goodreads(
                    &db,
                    &temp_path_clone,
                    &opts,
                    None,
                )?)
            }
            ImportSourceKind::StoryGraph => {
                let opts = toku_import::StorygraphImportOptions { dry_run: true };
                Ok(toku_import::import_storygraph(
                    &db,
                    &temp_path_clone,
                    &opts,
                    None,
                )?)
            }
            ImportSourceKind::Calibre { .. } => unreachable!(),
        }
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    // Create session
    let session = ImportSession {
        id: session_id.clone(),
        source: source_kind,
        file_path: temp_path,
        status: ImportSessionStatus::Preview,
        dry_run_report: Some(report),
        final_report: None,
        progress_tx: None,
        latest_progress: None,
    };

    state
        .import_sessions
        .lock()
        .map_err(|e| WebError::Internal(e.to_string()))?
        .insert(session_id.clone(), session);

    Ok(Redirect::to(&format!("/import/preview/{session_id}")))
}

/// `POST /import/calibre` — handle Calibre library path submission.
pub async fn submit_calibre_path(
    State(state): State<AppState>,
    form: axum::extract::Form<CalibreForm>,
) -> Result<Redirect, WebError> {
    let path = PathBuf::from(&form.path);

    // Validate: canonicalize and check metadata.db exists
    let canonical = path
        .canonicalize()
        .map_err(|_| WebError::Internal(format!("path not found: {}", form.path)))?;

    if !canonical.is_dir() {
        return Err(WebError::Internal(format!(
            "not a directory: {}",
            canonical.display()
        )));
    }

    let metadata_db = canonical.join("metadata.db");
    if !metadata_db.exists() {
        return Err(WebError::Internal(format!(
            "metadata.db not found in {}",
            canonical.display()
        )));
    }

    let import_covers = form.import_covers.is_some();
    let session_id = uuid::Uuid::now_v7().to_string();

    // Run dry-run
    let db_path = state.db_path.clone();
    let cal_path = canonical.clone();

    let report = tokio::task::spawn_blocking(move || -> Result<ImportReport, WebError> {
        let db = toku_db::Database::open_no_migrate_default(&db_path)?;
        let opts = toku_import::CalibreImportOptions {
            dry_run: true,
            import_covers,
        };
        Ok(toku_import::import_calibre(&db, &cal_path, &opts)?)
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    let session = ImportSession {
        id: session_id.clone(),
        source: ImportSourceKind::Calibre { import_covers },
        file_path: canonical,
        status: ImportSessionStatus::Preview,
        dry_run_report: Some(report),
        final_report: None,
        progress_tx: None,
        latest_progress: None,
    };

    state
        .import_sessions
        .lock()
        .map_err(|e| WebError::Internal(e.to_string()))?
        .insert(session_id.clone(), session);

    Ok(Redirect::to(&format!("/import/preview/{session_id}")))
}

#[derive(serde::Deserialize)]
pub struct CalibreForm {
    pub path: String,
    pub import_covers: Option<String>,
}

/// `GET /import/preview/{id}` — show dry-run preview.
pub async fn import_preview(
    State(state): State<AppState>,
    csrf: crate::auth::CsrfToken,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let sessions = state
        .import_sessions
        .lock()
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let session = sessions
        .get(&id)
        .ok_or_else(|| WebError::Internal("import session not found".into()))?;

    let report = session
        .dry_run_report
        .as_ref()
        .ok_or_else(|| WebError::Internal("no preview available".into()))?;

    Ok(Html(
        import_views::preview_page(&id, &session.source, report, csrf.value()).into_string(),
    ))
}

/// `POST /import/execute/{id}` — start the actual import, redirect to progress page.
pub async fn execute_import(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Redirect, WebError> {
    let (tx, source, file_path) = {
        let mut sessions = state
            .import_sessions
            .lock()
            .map_err(|e| WebError::Internal(e.to_string()))?;

        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| WebError::Internal("import session not found".into()))?;

        let (tx, _rx) = broadcast::channel::<ProgressEvent>(512);
        session.progress_tx = Some(tx.clone());
        session.status = ImportSessionStatus::Running;

        (
            tx.clone(),
            session.source.clone(),
            session.file_path.clone(),
        )
    }; // Mutex dropped here

    // Spawn the actual import in background
    let db_path = state.db_path.clone();
    let sessions = state.import_sessions.clone();
    let session_id = id.clone();
    let tx_complete = tx.clone();

    tokio::task::spawn_blocking(move || {
        let result = run_import(
            &db_path,
            &source,
            &file_path,
            tx.clone(),
            sessions.clone(),
            &session_id,
        );

        let mut sessions_guard = sessions.lock().unwrap();
        if let Some(session) = sessions_guard.get_mut(&session_id) {
            match result {
                Ok(report) => {
                    session.final_report = Some(report);
                    session.status = ImportSessionStatus::Complete;
                }
                Err(e) => {
                    session.status = ImportSessionStatus::Failed(e.to_string());
                }
            }
        }

        // Send terminal event
        let _ = tx_complete.send(ProgressEvent {
            event_type: "complete".to_string(),
            row: 0,
            total: 0,
            title: String::new(),
            imported: 0,
            skipped: 0,
            updated: 0,
            errors: 0,
        });
    });

    Ok(Redirect::to(&format!("/import/progress-page/{id}")))
}

/// `GET /import/progress-page/{id}` — render the progress page (HTML with SSE client).
pub async fn progress_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let sessions = state
        .import_sessions
        .lock()
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let session = sessions
        .get(&id)
        .ok_or_else(|| WebError::Internal("import session not found".into()))?;

    // If already complete, redirect to results
    if matches!(
        session.status,
        ImportSessionStatus::Complete | ImportSessionStatus::Failed(_)
    ) {
        drop(sessions);
        return Ok(Html(format!(
            r#"<meta http-equiv="refresh" content="0;url=/import/results/{id}">"#
        )));
    }

    Ok(Html(
        import_views::progress_page(&id, &session.source).into_string(),
    ))
}

/// `GET /import/progress/{id}` — SSE endpoint streaming progress events.
pub async fn import_progress_sse(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>>, WebError>
{
    let (rx, latest, status) = {
        let sessions = state
            .import_sessions
            .lock()
            .map_err(|e| WebError::Internal(e.to_string()))?;

        let session = sessions
            .get(&id)
            .ok_or_else(|| WebError::Internal("import session not found".into()))?;

        let rx = session.progress_tx.as_ref().map(|tx| tx.subscribe());
        let latest = session.latest_progress.clone();
        let status = session.status.clone();

        (rx, latest, status)
    };

    let is_done = matches!(
        status,
        ImportSessionStatus::Complete | ImportSessionStatus::Failed(_)
    );

    // Use an mpsc channel to unify replay + live events into one stream
    let (tx, mpsc_rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(128);

    // Spawn a task to feed events into the channel
    tokio::spawn(async move {
        // Replay latest progress
        if let Some(p) = latest {
            let json = serde_json::to_string(&p).unwrap_or_default();
            let _ = tx.send(Ok(Event::default().data(json))).await;
        }

        if is_done {
            let complete = complete_event();
            let json = serde_json::to_string(&complete).unwrap_or_default();
            let _ = tx.send(Ok(Event::default().data(json))).await;
            return;
        }

        // Stream live events from broadcast
        if let Some(rx) = rx {
            let mut stream = tokio_stream::wrappers::BroadcastStream::new(rx);
            while let Some(result) = stream.next().await {
                if let Ok(event) = result {
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(json))).await.is_err() {
                        break; // client disconnected
                    }
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(mpsc_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn complete_event() -> ProgressEvent {
    ProgressEvent {
        event_type: "complete".to_string(),
        row: 0,
        total: 0,
        title: String::new(),
        imported: 0,
        skipped: 0,
        updated: 0,
        errors: 0,
    }
}

/// `GET /import/results/{id}` — show final import results.
pub async fn import_results(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let sessions = state
        .import_sessions
        .lock()
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let session = sessions
        .get(&id)
        .ok_or_else(|| WebError::Internal("import session not found".into()))?;

    match &session.status {
        ImportSessionStatus::Complete => {
            let report = session
                .final_report
                .as_ref()
                .ok_or_else(|| WebError::Internal("no report available".into()))?;
            Ok(Html(
                import_views::results_page(&session.source, report).into_string(),
            ))
        }
        ImportSessionStatus::Failed(err) => {
            // Show error as a simple results page
            let mut report = ImportReport::default();
            report.error_details.push(err.clone());
            report.errors = 1;
            Ok(Html(
                import_views::results_page(&session.source, &report).into_string(),
            ))
        }
        _ => Err(WebError::Internal("import not yet complete".into())),
    }
}

// ── Import execution ────────────────────────────────────────────────

/// Run the actual import (called inside spawn_blocking).
fn run_import(
    db_path: &std::path::Path,
    source: &ImportSourceKind,
    file_path: &std::path::Path,
    tx: broadcast::Sender<ProgressEvent>,
    sessions: ImportSessions,
    session_id: &str,
) -> Result<ImportReport, WebError> {
    let db = toku_db::Database::open_no_migrate_default(db_path)?;

    match source {
        ImportSourceKind::Goodreads => {
            let opts = toku_import::GoodreadsImportOptions { dry_run: false };
            let mut observer = WebImportObserver {
                tx,
                sessions,
                session_id: session_id.to_string(),
                imported: 0,
                skipped: 0,
                updated: 0,
                errors: 0,
            };
            Ok(toku_import::import_goodreads(
                &db,
                file_path,
                &opts,
                Some(&mut observer),
            )?)
        }
        ImportSourceKind::StoryGraph => {
            let opts = toku_import::StorygraphImportOptions { dry_run: false };
            let mut observer = WebImportObserver {
                tx,
                sessions,
                session_id: session_id.to_string(),
                imported: 0,
                skipped: 0,
                updated: 0,
                errors: 0,
            };
            Ok(toku_import::import_storygraph(
                &db,
                file_path,
                &opts,
                Some(&mut observer),
            )?)
        }
        ImportSourceKind::Calibre { import_covers } => {
            let opts = toku_import::CalibreImportOptions {
                dry_run: false,
                import_covers: *import_covers,
            };
            Ok(toku_import::import_calibre(&db, file_path, &opts)?)
        }
    }
}
