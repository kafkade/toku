//! Web dashboard for Toku — serves the statistics UI and import wizard.
//!
//! Built with Axum and maud (server-side HTML). Charts are rendered as
//! inline SVG — no external JavaScript dependencies.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};

mod auth;
mod auth_handlers;
mod auth_views;
mod charts;
mod conflicts_handlers;
mod error;
mod handlers;
pub mod import_handlers;
mod import_views;
pub mod library_handlers;
mod library_views;
mod sync_handlers;
mod sync_status;
mod views;

pub use auth::WebMode;
pub use error::WebError;

/// Shared application state for Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub import_sessions: import_handlers::ImportSessions,
    pub temp_dir: PathBuf,
    pub covers_dir: PathBuf,
    /// Authentication mode (local = no auth, hosted = auth required).
    pub mode: WebMode,
    /// Whether to mark auth cookies `Secure` (disable for plain-HTTP testing).
    pub secure_cookies: bool,
}

/// Routes that render the dashboard itself (gated behind auth in hosted mode).
fn dashboard_routes() -> Router<AppState> {
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
        // Sync conflicts
        .route("/conflicts", get(conflicts_handlers::conflicts_page))
        .route(
            "/conflicts/resolve/{id}",
            post(conflicts_handlers::resolve_conflict),
        )
        .route(
            "/conflicts/resolve-all",
            post(conflicts_handlers::resolve_all_conflicts),
        )
        // Sync status
        .route("/sync", get(sync_handlers::sync_page))
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
}

/// Build the Axum router for the web dashboard.
pub fn build_router(
    db_path: PathBuf,
    temp_dir: PathBuf,
    mode: WebMode,
    secure_cookies: bool,
) -> Router {
    let covers_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("covers");

    // Record the data dir so the header sync badge can read sync state.
    if let Some(data_dir) = db_path.parent() {
        sync_status::set_data_dir(data_dir.to_path_buf());
    }

    let state = AppState {
        db_path,
        import_sessions: Arc::new(Mutex::new(HashMap::new())),
        temp_dir,
        covers_dir,
        mode,
        secure_cookies,
    };

    auth::set_hosted(mode.is_hosted());

    let dashboard = dashboard_routes();

    if mode.is_hosted() {
        // Public, unauthenticated routes.
        let public = Router::new()
            .route(
                "/login",
                get(auth_handlers::login_page).post(auth_handlers::login_submit),
            )
            .route(
                "/setup",
                get(auth_handlers::setup_page).post(auth_handlers::setup_submit),
            )
            .route(
                "/logout",
                get(auth_handlers::logout).post(auth_handlers::logout),
            )
            .route("/healthz", get(auth_handlers::healthz));

        // Auth gate on the dashboard routes only; CSRF protection on everything.
        let protected = dashboard.route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

        protected
            .merge(public)
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth::csrf_protect,
            ))
            .with_state(state)
    } else {
        dashboard
            .route("/healthz", get(auth_handlers::healthz))
            .with_state(state)
    }
}

/// Start serving using a pre-bound listener (local mode; used by the desktop app).
///
/// Runs migrations once at startup, then serves the dashboard on the
/// listener's local address. Use this when you need to control the
/// listener (e.g. binding to a random port for the desktop app).
pub async fn serve_on(db_path: PathBuf, listener: tokio::net::TcpListener) -> Result<(), WebError> {
    serve_on_with(db_path, listener, WebMode::Local, false).await
}

/// Start serving using a pre-bound listener with an explicit mode.
pub async fn serve_on_with(
    db_path: PathBuf,
    listener: tokio::net::TcpListener,
    mode: WebMode,
    secure_cookies: bool,
) -> Result<(), WebError> {
    // Run migrations once at startup
    toku_db::Database::open(&db_path)
        .map_err(|e| WebError::Internal(format!("failed to open database: {e}")))?;

    // Create temp directory for uploads
    let temp_dir = std::env::temp_dir().join("toku-web-uploads");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| WebError::Internal(format!("failed to create temp dir: {e}")))?;

    let app = build_router(db_path, temp_dir, mode, secure_cookies);

    axum::serve(listener, app)
        .await
        .map_err(|e| WebError::Internal(format!("server error: {e}")))?;

    Ok(())
}

/// Start the web dashboard server.
///
/// Runs migrations once, then serves the dashboard on `{host}:{port}`.
pub async fn serve(
    db_path: PathBuf,
    host: &str,
    port: u16,
    mode: WebMode,
    secure_cookies: bool,
) -> Result<(), WebError> {
    // In hosted mode the server is meant to face a network; in local mode we
    // refuse anything but loopback to preserve the single-user, no-auth posture.
    if mode == WebMode::Local && !is_loopback_host(host) {
        return Err(WebError::Internal(format!(
            "refusing to bind {host} without authentication: local mode is loopback-only. \
             Use --hosted to enable authentication for network access."
        )));
    }

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| WebError::Internal(format!("failed to bind {addr}: {e}")))?;

    match mode {
        WebMode::Hosted => {
            eprintln!("Toku dashboard (hosted) → http://{addr}/  — authentication required")
        }
        WebMode::Local => eprintln!("Toku dashboard → http://{addr}/library"),
    }

    serve_on_with(db_path, listener, mode, secure_cookies).await
}

/// True when the host string is a loopback address or `localhost`.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}
