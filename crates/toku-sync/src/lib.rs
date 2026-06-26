//! Toku sync relay server library.
//!
//! The server is primarily a binary (`main.rs`), but the router and supporting
//! modules are exposed here so they can be started in-process — most notably by
//! the multi-device integration tests, which spin up a real Axum server on a
//! random port and drive it through the real sync client.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod handlers;
pub mod models;

use std::path::PathBuf;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post};
use tower_http::trace::TraceLayer;

/// Build the sync server's Axum router, backed by the SQLite database at
/// `db_path`. The database must already exist with migrations applied (see
/// [`db::SyncDatabase::open`]).
pub fn build_router(db_path: PathBuf) -> Router {
    // Routes that require authentication
    let authenticated = Router::new()
        .route("/api/v1/devices", get(handlers::list_devices))
        .route("/api/v1/devices/{id}", delete(handlers::delete_device))
        .route("/api/v1/push", post(handlers::push_ops))
        .route("/api/v1/pull", get(handlers::pull_ops))
        .route("/api/v1/pull/all", get(handlers::pull_all_ops))
        .route("/api/v1/salt", get(handlers::get_salt))
        .route("/api/v1/snapshot", get(handlers::download_snapshot))
        .route("/api/v1/snapshot", post(handlers::upload_snapshot))
        .route("/api/v1/rekey", post(handlers::rekey))
        .layer(middleware::from_fn_with_state(
            db_path.clone(),
            auth::require_auth,
        ));

    // Public routes (unauthenticated)
    Router::new()
        .route("/health", get(handlers::health))
        // Legacy passwordless registration
        .route("/api/v1/register", post(handlers::register))
        // SRP-6a authentication endpoints
        .route("/api/v1/auth/enroll", post(auth::srp_enroll))
        .route("/api/v1/auth/challenge", post(auth::srp_challenge))
        .route("/api/v1/auth/verify", post(auth::srp_verify))
        .merge(authenticated)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50 MB (rekey may be large)
        .layer(TraceLayer::new_for_http())
        .with_state(db_path)
}
