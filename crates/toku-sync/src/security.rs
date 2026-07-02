//! Security-hardening helpers for issue #160 (threat model #125):
//!
//! - [`server_secret`] — a lazily-generated, persisted high-entropy secret.
//! - [`phantom_credentials`] — deterministic pseudo SRP salt+verifier derived
//!   from the server secret for unknown/disabled accounts, so the account
//!   challenge stage does not leak whether an email exists (F5).
//! - [`audit`] — append a row to the `audit_log` table and emit a `tracing`
//!   event (F7).
//! - [`RateLimiter`] — a lightweight in-process, per-IP + global fixed-window
//!   limiter for the authentication endpoints (F8).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ConnectInfo;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use srp::ClientG2048;

use crate::db::SyncDatabase;
use crate::error::SyncError;

/// Fetch the server secret, generating and persisting one on first use.
///
/// Stored hex-encoded in `server_config` under the `server_secret` key. Used
/// only to derive phantom SRP credentials; it never touches user data and its
/// disclosure would at worst let an attacker distinguish phantom from real
/// accounts (the pre-existing enumeration risk this mitigates).
pub fn server_secret(db: &SyncDatabase) -> Result<Vec<u8>, SyncError> {
    if let Ok(hex_secret) = db.conn.query_row(
        "SELECT value FROM server_config WHERE key = 'server_secret'",
        [],
        |row| row.get::<_, String>(0),
    ) && let Ok(bytes) = hex::decode(&hex_secret)
    {
        return Ok(bytes);
    }
    // Generate a fresh 32-byte secret and persist it (idempotent: another
    // request may race us, so ignore on conflict and re-read).
    let mut bytes = [0u8; 32];
    rand::RngExt::fill(&mut rand::rng(), &mut bytes);
    let hex_secret = hex::encode(bytes);
    db.conn.execute(
        "INSERT OR IGNORE INTO server_config (key, value) VALUES ('server_secret', ?1)",
        [&hex_secret],
    )?;

    let stored: String = db.conn.query_row(
        "SELECT value FROM server_config WHERE key = 'server_secret'",
        [],
        |row| row.get(0),
    )?;
    hex::decode(&stored).map_err(|_| SyncError::Internal("server secret is corrupt".into()))
}

/// Domain-separated derivation of pseudo-random bytes from the server secret.
fn derive(secret: &[u8], domain: &str, identity: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(secret);
    hasher.update(domain.as_bytes());
    hasher.update([0u8]);
    hasher.update(identity.as_bytes());
    hasher.finalize().to_vec()
}

/// Derive a deterministic phantom `(srp_salt_hex, srp_verifier_hex)` for an
/// identity that has no real (active) account.
///
/// The salt and verifier are stable per identity (so repeated challenges look
/// consistent, like a real account) and shaped identically to real SRP-6a
/// material, so a client cannot distinguish this from a genuine challenge. No
/// real password can ever satisfy the resulting verifier, so `verify` fails.
pub fn phantom_credentials(secret: &[u8], identity: &str) -> (String, String) {
    let salt = derive(secret, "srp-salt", identity);
    let salt = &salt[..16]; // 128-bit salt, matching real accounts
    let password_seed = derive(secret, "srp-verifier", identity);

    let client = ClientG2048::<Sha256>::new();
    let verifier = client.compute_verifier(identity.as_bytes(), &password_seed, salt);

    (hex::encode(salt), hex::encode(verifier))
}

/// Append a security event to the audit log and emit a matching `tracing`
/// event. Best-effort: a logging failure never breaks the request path.
pub fn audit(
    db: &SyncDatabase,
    event_type: &str,
    actor: Option<&str>,
    target: Option<&str>,
    outcome: &str,
    detail: Option<&str>,
    ip: Option<&str>,
) {
    let _ = db.conn.execute(
        "INSERT INTO audit_log (event_type, actor, target, outcome, detail, ip)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![event_type, actor, target, outcome, detail, ip],
    );
    tracing::info!(
        target: "toku_sync::audit",
        event = event_type,
        actor = actor.unwrap_or("-"),
        subject = target.unwrap_or("-"),
        outcome,
        detail = detail.unwrap_or("-"),
        "security audit event"
    );
}

// ── Rate limiting (F8) ───────────────────────────────────────────────────────

/// Default requests allowed per client IP within [`RateLimiter::window`].
const DEFAULT_PER_IP_MAX: u32 = 60;
/// Default total requests allowed across all clients within the window.
const DEFAULT_GLOBAL_MAX: u32 = 600;
/// Default fixed-window length.
const DEFAULT_WINDOW_SECS: u64 = 60;

/// A minimal in-process, fixed-window rate limiter keyed by client IP with an
/// additional global ceiling. One instance is created per router so it is
/// isolated between tests and between server processes.
///
/// This is defense-in-depth *in the app*; operators should still front the
/// server with a rate-limiting reverse proxy (documented in `sync-server.md`).
pub struct RateLimiter {
    inner: Mutex<Window>,
    per_ip_max: u32,
    global_max: u32,
    window: Duration,
}

struct Window {
    started: Instant,
    global_count: u32,
    per_ip: HashMap<IpAddr, u32>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(
            DEFAULT_PER_IP_MAX,
            DEFAULT_GLOBAL_MAX,
            Duration::from_secs(DEFAULT_WINDOW_SECS),
        )
    }
}

impl RateLimiter {
    /// Create a limiter with explicit limits.
    pub fn new(per_ip_max: u32, global_max: u32, window: Duration) -> Self {
        Self {
            inner: Mutex::new(Window {
                started: Instant::now(),
                global_count: 0,
                per_ip: HashMap::new(),
            }),
            per_ip_max,
            global_max,
            window,
        }
    }

    /// Record a request from `ip`. Returns `Some(retry_after_secs)` when the
    /// request should be rejected, or `None` when it is allowed.
    pub fn check(&self, ip: IpAddr) -> Option<u64> {
        let mut w = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = w.started.elapsed();
        if elapsed >= self.window {
            w.started = Instant::now();
            w.global_count = 0;
            w.per_ip.clear();
        }

        let retry_after = self.window.saturating_sub(w.started.elapsed()).as_secs() + 1;

        if w.global_count >= self.global_max {
            return Some(retry_after);
        }
        let entry = w.per_ip.entry(ip).or_insert(0);
        if *entry >= self.per_ip_max {
            return Some(retry_after);
        }

        *entry += 1;
        w.global_count += 1;
        None
    }
}

/// Best-effort client IP: the first `X-Forwarded-For` hop when present (trusted
/// reverse-proxy deployments), otherwise the peer address from `ConnectInfo`.
/// Falls back to `0.0.0.0` when neither is available (e.g. in unit tests using
/// `oneshot`), which simply shares one bucket.
fn client_ip(req: &Request) -> IpAddr {
    if let Some(xff) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return ip;
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Axum middleware enforcing [`RateLimiter`]; returns HTTP 429 when exceeded.
pub async fn rate_limit(
    limiter: Arc<RateLimiter>,
    req: Request,
    next: Next,
) -> Result<Response, SyncError> {
    let ip = client_ip(&req);
    if let Some(retry_after) = limiter.check(ip) {
        return Err(SyncError::RateLimited {
            retry_after: format!("{retry_after}s"),
        });
    }
    Ok(next.run(req).await)
}
