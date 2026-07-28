//! Managed-tier runtime wiring (issue #206, ADR-014).
//!
//! [`ManagedRuntime`] carries the process-level, opt-in managed-tier
//! capabilities that cannot live in the database: the outbound [`Mailer`] used
//! for signup verification and the in-memory per-user [`UserRateLimiter`]. It is
//! attached to the router as an axum `Extension` so handlers and middleware can
//! read it.
//!
//! The [`Default`] impl is the **self-hosted** configuration — a logging-only
//! mailer and a disabled per-user limiter — so `build_router` (and therefore the
//! test harness and every existing caller) behaves exactly as before. A managed
//! operator constructs a configured runtime via [`ManagedRuntime::new`].
//!
//! Storage- and op-count quotas and the `require_email_verification` toggle are
//! deliberately **not** here: they live in `instance_config` / `user_quota` and
//! are read from the database per request, so they need no process wiring.

use std::sync::Arc;
use std::time::Duration;

use crate::mailer::{LoggingMailer, Mailer};
use crate::security::UserRateLimiter;

/// Process-level managed-tier capabilities, shared across handlers via an axum
/// `Extension`. Cheap to clone (everything is `Arc`-backed).
#[derive(Clone)]
pub struct ManagedRuntime {
    /// Outbound mail transport for signup verification links.
    pub mailer: Arc<dyn Mailer>,
    /// Public base URL used to build verification links (e.g.
    /// `https://sync.example.com`). `None` when unconfigured.
    pub public_base_url: Option<String>,
    /// Per-authenticated-user rate limiter (disabled by default).
    pub user_rate_limiter: Arc<UserRateLimiter>,
}

impl Default for ManagedRuntime {
    fn default() -> Self {
        Self {
            mailer: Arc::new(LoggingMailer),
            public_base_url: None,
            user_rate_limiter: Arc::new(UserRateLimiter::new(0, Duration::from_secs(60))),
        }
    }
}

impl ManagedRuntime {
    /// Build a configured managed runtime. A `per_user_rate_max` of `0` leaves
    /// the per-user limiter disabled.
    pub fn new(
        mailer: Arc<dyn Mailer>,
        public_base_url: Option<String>,
        per_user_rate_max: u32,
        per_user_rate_window: Duration,
    ) -> Self {
        Self {
            mailer,
            public_base_url,
            user_rate_limiter: Arc::new(UserRateLimiter::new(
                per_user_rate_max,
                per_user_rate_window,
            )),
        }
    }
}
