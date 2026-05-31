//! Web dashboard for Toku — serves the statistics UI and import wizard.
//!
//! Built with Axum and maud (server-side HTML). Charts are rendered as
//! inline SVG — no external JavaScript dependencies.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};

mod charts;
mod error;
mod handlers;
pub mod import_handlers;
mod import_views;
pub mod library_handlers;
mod library_views;
mod views;

pub use error::WebError;

/// Shared application state for Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub import_sessions: import_handlers::ImportSessions,
    pub temp_dir: PathBuf,
    pub covers_dir: PathBuf,
}

/// Build the Axum router for the web dashboard.
pub fn build_router(db_path: PathBuf, temp_dir: PathBuf) -> Router {
    let covers_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("covers");

    let state = AppState {
        db_path,
        import_sessions: Arc::new(Mutex::new(HashMap::new())),
        temp_dir,
        covers_dir,
    };

    Router::new()
        // Root
        .route("/", get(handlers::root))
        // Library
        .route("/library", get(library_handlers::library_page))
        .route("/books/{id}", get(library_handlers::book_detail))
        .route("/search", get(library_handlers::search_page))
        .route("/covers/{hash}", get(library_handlers::serve_cover))
        // Stats
        .route("/stats", get(handlers::stats_dashboard))
        .route("/stats/wrap/{year}", get(handlers::yearly_wrap))
        .route("/api/stats", get(handlers::stats_json))
        // Import wizard
        .route("/import", get(import_handlers::import_page))
        .route("/import/upload", post(import_handlers::upload_csv))
        .route(
            "/import/calibre",
            post(import_handlers::submit_calibre_path),
        )
        .route("/import/preview/{id}", get(import_handlers::import_preview))
        .route(
            "/import/execute/{id}",
            post(import_handlers::execute_import),
        )
        .route(
            "/import/progress-page/{id}",
            get(import_handlers::progress_page),
        )
        .route(
            "/import/progress/{id}",
            get(import_handlers::import_progress_sse),
        )
        .route("/import/results/{id}", get(import_handlers::import_results))
        .with_state(state)
}

/// Start serving using a pre-bound listener.
///
/// Runs migrations once at startup, then serves the dashboard on the
/// listener's local address. Use this when you need to control the
/// listener (e.g. binding to a random port for the desktop app).
pub async fn serve_on(db_path: PathBuf, listener: tokio::net::TcpListener) -> Result<(), WebError> {
    // Run migrations once at startup
    toku_db::Database::open(&db_path)
        .map_err(|e| WebError::Internal(format!("failed to open database: {e}")))?;

    // Create temp directory for uploads
    let temp_dir = std::env::temp_dir().join("toku-web-uploads");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| WebError::Internal(format!("failed to create temp dir: {e}")))?;

    let app = build_router(db_path, temp_dir);

    axum::serve(listener, app)
        .await
        .map_err(|e| WebError::Internal(format!("server error: {e}")))?;

    Ok(())
}

/// Start the web dashboard server.
///
/// Runs migrations once, then serves the dashboard on `{host}:{port}`.
pub async fn serve(db_path: PathBuf, host: &str, port: u16) -> Result<(), WebError> {
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| WebError::Internal(format!("failed to bind {addr}: {e}")))?;

    eprintln!("Toku dashboard → http://{addr}/library");

    serve_on(db_path, listener).await
}
