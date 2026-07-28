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
pub mod mailer;
pub mod managed;
pub mod models;
pub mod protocol;
pub mod security;

use std::path::PathBuf;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post};
use tower_http::trace::TraceLayer;

pub use managed::ManagedRuntime;

/// Build the sync server's Axum router with the **self-hosted** default managed
/// runtime (no per-user rate limit, logging-only mailer). Storage/op quotas and
/// email verification remain governed by `instance_config`, which defaults to
/// unlimited / disabled. Every existing caller and test keeps today's behaviour.
pub fn build_router(db_path: PathBuf) -> Router {
    build_router_with(db_path, ManagedRuntime::default())
}

/// Build the sync server's Axum router, backed by the SQLite database at
/// `db_path`, with an explicit [`ManagedRuntime`] carrying opt-in managed-tier
/// capabilities (per-user rate limiting, SMTP verification delivery). The
/// database must already exist with migrations applied (see
/// [`db::SyncDatabase::open`]).
pub fn build_router_with(db_path: PathBuf, managed: ManagedRuntime) -> Router {
    // Per-authenticated-user rate limiter (ADR-014 D3), disabled by default.
    // Applied as an *inner* layer to the auth middleware so it runs after the
    // owning identity is resolved into the request extensions.
    let user_limiter = managed.user_rate_limiter.clone();
    let per_user_rate = {
        let user_limiter = user_limiter.clone();
        move |req, next| {
            let limiter = user_limiter.clone();
            async move { security::user_rate_limit(limiter, req, next).await }
        }
    };

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
        .route("/api/v1/auth/logout", post(handlers::device_logout))
        .layer(middleware::from_fn(per_user_rate.clone()))
        .layer(middleware::from_fn_with_state(
            db_path.clone(),
            auth::require_auth,
        ));

    // Account/admin routes requiring a *user* session (issues #119, #120).
    let user_authenticated = Router::new()
        .route("/api/v1/admin/users", get(handlers::list_users))
        .route(
            "/api/v1/admin/users/{id}/status",
            post(handlers::set_user_status),
        )
        // Per-user encrypted backup / restore (issue #206, ADR-014 D6). Operator
        // (admin) scoped; ciphertext + opaque metadata only, ZK preserved.
        .route(
            "/api/v1/admin/users/{id}/backup",
            get(handlers::admin_backup_user),
        )
        .route(
            "/api/v1/admin/users/{id}/backup/restore",
            post(handlers::admin_restore_user),
        )
        .route(
            "/api/v1/admin/registration",
            get(handlers::get_registration).put(handlers::set_registration),
        )
        .route(
            "/api/v1/admin/device-approvals",
            get(handlers::get_device_approvals).put(handlers::set_device_approvals),
        )
        // Authenticated, Secret-Key-gated device enrollment (issue #120).
        .route("/api/v1/devices/enroll", post(handlers::enroll_device))
        .route(
            "/api/v1/devices/{id}/approval",
            post(handlers::approve_device),
        )
        .route(
            "/api/v1/devices/{id}/session",
            post(handlers::create_device_session),
        )
        .route(
            "/api/v1/account/devices",
            get(handlers::list_account_devices),
        )
        .route(
            "/api/v1/account/devices/{id}",
            delete(handlers::delete_account_device),
        )
        // Zero-knowledge account key bundle for multi-device recovery (#143).
        .route("/api/v1/account/keys", get(handlers::account_keys))
        .route("/api/v1/account/logout", post(handlers::account_logout))
        .layer(middleware::from_fn(per_user_rate.clone()))
        .layer(middleware::from_fn_with_state(
            db_path.clone(),
            auth::require_user_auth,
        ));

    // Everything except /health is gated on protocol version (#126).
    // The authentication endpoints additionally sit behind an app-level rate
    // limiter (F8); one limiter instance per router keeps it isolated.
    let limiter = std::sync::Arc::new(security::RateLimiter::default());
    let auth_endpoints = Router::new()
        // Legacy passwordless registration
        .route("/api/v1/register", post(handlers::register))
        // SRP-6a authentication endpoints
        .route("/api/v1/auth/enroll", post(auth::srp_enroll))
        .route("/api/v1/auth/challenge", post(auth::srp_challenge))
        .route("/api/v1/auth/verify", post(auth::srp_verify))
        // User account (SRP) endpoints
        .route("/api/v1/account/signup", post(auth::account_signup))
        .route("/api/v1/account/challenge", post(auth::account_challenge))
        .route("/api/v1/account/verify", post(auth::account_verify))
        // Self-serve signup email verification (issue #206, ADR-014 D4).
        .route("/api/v1/account/verify-email", post(auth::verify_email))
        .route(
            "/api/v1/account/resend-verification",
            post(auth::resend_verification),
        )
        .layer(middleware::from_fn(move |req, next| {
            let limiter = limiter.clone();
            async move { security::rate_limit(limiter, req, next).await }
        }));

    let gated = auth_endpoints
        .merge(authenticated)
        .merge(user_authenticated)
        .layer(middleware::from_fn_with_state(
            db_path.clone(),
            protocol::require_protocol,
        ));

    // Public routes (unauthenticated). Health stays ungated so clients can probe
    // protocol versions before being turned away.
    Router::new()
        .route("/health", get(handlers::health))
        .merge(gated)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50 MB (rekey may be large)
        // Managed-tier capabilities available to every handler (issue #206).
        .layer(axum::Extension(managed))
        .layer(TraceLayer::new_for_http())
        .with_state(db_path)
}
