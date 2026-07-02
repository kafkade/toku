//! Hosted-mode authentication for the web dashboard.
//!
//! `toku serve` runs in one of two modes (see [`WebMode`]):
//!
//! - **Local** (default): no authentication, binds loopback only. This preserves
//!   the historical single-user behaviour where the dashboard renders the local
//!   SQLite directly for one trusted user on their own machine.
//! - **Hosted**: every route is gated behind a cookie session. The first run has
//!   no admin account, so all requests are redirected to `/setup` to create one
//!   (Secret Key + Emergency Kit). Subsequent requests are redirected to `/login`
//!   until they present a valid session cookie.
//!
//! # Trusted-server trade-off (flag for threat model #125)
//!
//! The dashboard renders **server-side HTML from the decrypted local library**,
//! so in hosted mode the server process necessarily holds plaintext. Login sends
//! the password to the server over TLS, which recomputes the SRP verifier and
//! compares it in constant time. This is the *trusted-server* posture: the
//! zero-knowledge guarantees of ADR-010 apply to the sync relay, not to a
//! dashboard you intentionally point at your own decrypted library. True
//! in-browser (zero-knowledge) unlock would require client-side WASM crypto and
//! is deliberately out of scope for issue #122.

use std::path::Path;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use rand::RngExt;
use sha2::{Digest, Sha256};
use srp::ClientG2048;
use subtle::ConstantTimeEq;

use crate::AppState;

/// Name of the session cookie holding the opaque session token.
pub const SESSION_COOKIE: &str = "toku_session";
/// Name of the CSRF double-submit cookie.
pub const CSRF_COOKIE: &str = "toku_csrf";
/// Form field / query parameter carrying the CSRF token.
pub const CSRF_FIELD: &str = "csrf_token";

/// Session lifetime in hours.
const SESSION_TTL_HOURS: i64 = 24;
/// Failed logins before the account is locked.
const MAX_FAILED_ATTEMPTS: i64 = 5;
/// Lockout duration in minutes.
const LOCKOUT_MINUTES: i64 = 15;
/// Maximum buffered body size when extracting a CSRF field (64 KiB).
const CSRF_BODY_LIMIT: usize = 64 * 1024;

/// Operating mode for the dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebMode {
    /// No authentication, loopback only (historical single-user behaviour).
    Local,
    /// Authentication required; first-run onboarding then login.
    Hosted,
}

impl WebMode {
    /// True when authentication should be enforced.
    pub fn is_hosted(self) -> bool {
        matches!(self, WebMode::Hosted)
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-wide flag mirroring the current [`WebMode`], so the shared page
/// layout (which has no access to request state) can render the "Sign out"
/// affordance only in hosted mode.
static HOSTED: AtomicBool = AtomicBool::new(false);

/// Record whether the server is running in hosted mode.
pub fn set_hosted(hosted: bool) {
    HOSTED.store(hosted, Ordering::Relaxed);
}

/// True when the server is running in hosted (authenticated) mode.
pub fn is_hosted_global() -> bool {
    HOSTED.load(Ordering::Relaxed)
}

/// The CSRF token for the current request, injected by [`csrf_protect`]. In
/// local mode (no CSRF middleware) this resolves to an empty token and forms
/// omit the hidden field.
#[derive(Clone, Debug, Default)]
pub struct CsrfToken(pub String);

impl CsrfToken {
    /// The token string, empty when CSRF protection is disabled.
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CsrfToken {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<CsrfToken>()
            .cloned()
            .unwrap_or_default())
    }
}

// ── Crypto / token helpers ───────────────────────────────────────────────────

/// SHA-256 a string and hex-encode the digest.
pub fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// Generate a 256-bit CSPRNG token, base64url (no pad) encoded.
pub fn generate_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate a fresh 16-byte SRP salt, hex-encoded.
fn generate_salt_hex() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Compute the hex-encoded SRP-6a verifier for an account.
///
/// The SRP identity is the account email; the SRP password input folds in the
/// account Secret Key (ADR-010 two-secret auth) via
/// [`toku_core::srp_verifier_input`] — matching toku-sync's account flow so the
/// two tiers stay interoperable. Deterministic for a given
/// (email, password, secret_key, salt).
fn compute_verifier_hex(email: &str, password: &str, secret_key: &[u8], salt: &[u8]) -> String {
    let client = ClientG2048::<Sha256>::new();
    let verifier_input = toku_core::srp_verifier_input(Some(secret_key), password);
    hex::encode(client.compute_verifier(email.as_bytes(), &verifier_input, salt))
}

// ── Account / session persistence (blocking; call via spawn_blocking) ─────────

/// Outcome of creating the first admin account.
pub struct AdminCreated {
    /// Formatted Secret Key (`TK-…`) to surface once in the Emergency Kit.
    pub secret_key: String,
    /// The account email.
    pub email: String,
}

/// True when at least one admin account exists.
pub fn admin_exists(db_path: &Path) -> Result<bool, String> {
    let db = open(db_path)?;
    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM web_users WHERE role = 'admin'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

/// Create the first-run admin account, returning its Emergency Kit material.
///
/// Generates a Secret Key, derives the account key hierarchy (so the Secret Key
/// gates the account per ADR-010) and an SRP verifier from the password, and
/// persists the user as `admin`. Refuses if an admin already exists.
pub fn create_admin(db_path: &Path, email: &str, password: &str) -> Result<AdminCreated, String> {
    let email = email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err("a valid email is required".into());
    }
    if password.len() < 8 {
        return Err("password must be at least 8 characters".into());
    }

    let secret_key = toku_core::SecretKey::generate().map_err(|e| e.to_string())?;
    let (account_keys, _data_key) = toku_core::AccountKeys::create(password, secret_key.as_bytes())
        .map_err(|e| e.to_string())?;

    let salt_hex = generate_salt_hex();
    let salt_bytes = hex::decode(&salt_hex).map_err(|e| e.to_string())?;
    let verifier_hex = compute_verifier_hex(email, password, secret_key.as_bytes(), &salt_bytes);

    let wrapped_private_key =
        serde_json::to_string(&account_keys.wrapped_private_key).map_err(|e| e.to_string())?;
    let kdf_params = serde_json::to_string(&account_keys.kdf).map_err(|e| e.to_string())?;
    let account_public_key = account_keys.public_key.clone();

    let user_id = uuid::Uuid::now_v7().to_string();

    let db = open(db_path)?;
    let tx = db.conn.unchecked_transaction().map_err(|e| e.to_string())?;

    let existing_admins: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM web_users WHERE role = 'admin'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if existing_admins > 0 {
        return Err("an administrator account already exists".into());
    }

    tx.execute(
        "INSERT INTO web_users
         (id, email, srp_salt, srp_verifier, wrapped_private_key,
          account_public_key, kdf_params, role, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'admin', 'active', datetime('now'))",
        rusqlite::params![
            user_id,
            email,
            salt_hex,
            verifier_hex,
            wrapped_private_key,
            account_public_key,
            kdf_params,
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(AdminCreated {
        secret_key: secret_key.format(),
        email: email.to_string(),
    })
}

/// Result of a login attempt.
pub enum LoginOutcome {
    /// Authenticated; carries a freshly issued session token (raw, for the cookie).
    Success { session_token: String },
    /// Wrong email/password.
    Invalid,
    /// Account locked due to too many failed attempts.
    Locked,
}

/// Verify credentials and, on success, issue a new session (session fixation is
/// avoided because a brand-new token is always minted here).
///
/// The SRP verifier folds in the account Secret Key (ADR-010 two-secret auth),
/// so `secret_key` is the raw user-entered key string (`TK-…`). A malformed
/// Secret Key is treated as invalid credentials, but the verifier computation
/// still runs (with placeholder key bytes) so timing does not distinguish a
/// bad key from a bad password.
///
/// Anti-enumeration (threat model #125, finding F5): an unknown email and an
/// inactive account are processed down the *same* path as a real account with a
/// wrong password — a verifier is always computed and compared in constant time
/// against a fixed dummy verifier — so response content and timing do not reveal
/// whether the email exists.
pub fn login(
    db_path: &Path,
    email: &str,
    password: &str,
    secret_key: &str,
) -> Result<LoginOutcome, String> {
    // A fixed dummy salt/verifier pair used when the account is absent or
    // inactive, so the SRP verifier computation always runs (uniform timing).
    // The verifier is 256 bytes (2048-bit group) of zero → never matches a real
    // `g^x mod N`, and matches the real verifier's hex length for a constant-time
    // compare that does not short-circuit on length.
    const DUMMY_SALT_HEX: &str = "00000000000000000000000000000000";
    const DUMMY_VERIFIER_HEX: &str = "0"; // expanded to full width below

    // Parse the Secret Key up front. A malformed key never authenticates, but we
    // still run the full verifier computation below (with placeholder bytes) so
    // the timing/response is identical to a wrong-password attempt.
    let parsed_key = toku_core::SecretKey::parse(secret_key.trim()).ok();
    let secret_key_bytes: [u8; 16] = parsed_key
        .as_ref()
        .map(|k| *k.as_bytes())
        .unwrap_or([0u8; 16]);
    let secret_key_valid = parsed_key.is_some();

    let email = email.trim();
    let db = open(db_path)?;

    let row: Option<(String, String, String, String, i64, Option<String>)> = db
        .conn
        .query_row(
            "SELECT id, srp_salt, srp_verifier, status, failed_attempts, locked_until
             FROM web_users WHERE email = ?1",
            [email],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .ok();

    // Resolve the (real or dummy) credentials to compare against without an
    // early return, so every branch performs the verifier computation.
    let real_active = row
        .as_ref()
        .map(|(_, _, _, status, _, _)| status == "active")
        .unwrap_or(false);

    let (user_id, salt_hex, verifier_hex, failed_attempts, locked_until) = match &row {
        Some((id, salt, verifier, status, failed, locked)) if status == "active" => (
            Some(id.clone()),
            salt.clone(),
            verifier.clone(),
            *failed,
            locked.clone(),
        ),
        _ => (
            None,
            DUMMY_SALT_HEX.to_string(),
            DUMMY_VERIFIER_HEX.repeat(512),
            0,
            None,
        ),
    };

    // Locked real accounts short-circuit to `Locked` (lockout inherently reveals
    // existence and is accepted by the threat model); unknown/inactive accounts
    // never reach here because `locked_until` is `None` for them.
    if real_active && let Some(until) = &locked_until {
        let still_locked: bool = db
            .conn
            .query_row("SELECT ?1 > datetime('now')", [until], |r| r.get(0))
            .unwrap_or(false);
        if still_locked {
            return Ok(LoginOutcome::Locked);
        }
    }

    let salt_bytes = hex::decode(&salt_hex).map_err(|e| e.to_string())?;
    let expected = compute_verifier_hex(email, password, &secret_key_bytes, &salt_bytes);

    let matches: bool = expected.as_bytes().ct_eq(verifier_hex.as_bytes()).into();
    if !matches || !real_active || !secret_key_valid {
        // Only real, active accounts accrue lockout state.
        if let Some(user_id) = &user_id {
            record_failure(&db, user_id, failed_attempts)?;
        }
        return Ok(LoginOutcome::Invalid);
    }

    let user_id = user_id.expect("real_active implies a user id");

    // Reset failure counter and mint a fresh session.
    db.conn
        .execute(
            "UPDATE web_users SET failed_attempts = 0, locked_until = NULL WHERE id = ?1",
            [&user_id],
        )
        .map_err(|e| e.to_string())?;

    let token = generate_token();
    let token_hash = sha256_hex(&token);
    db.conn
        .execute(
            "INSERT INTO web_sessions (token_hash, user_id, expires_at, created_at)
             VALUES (?1, ?2, datetime('now', ?3), datetime('now'))",
            rusqlite::params![token_hash, user_id, format!("+{SESSION_TTL_HOURS} hours")],
        )
        .map_err(|e| e.to_string())?;

    Ok(LoginOutcome::Success {
        session_token: token,
    })
}

/// Increment the failure counter, locking the account at the threshold.
fn record_failure(db: &toku_db::Database, user_id: &str, current: i64) -> Result<(), String> {
    let next = current + 1;
    if next >= MAX_FAILED_ATTEMPTS {
        db.conn
            .execute(
                "UPDATE web_users
                 SET failed_attempts = 0, locked_until = datetime('now', ?2)
                 WHERE id = ?1",
                rusqlite::params![user_id, format!("+{LOCKOUT_MINUTES} minutes")],
            )
            .map_err(|e| e.to_string())?;
    } else {
        db.conn
            .execute(
                "UPDATE web_users SET failed_attempts = ?2 WHERE id = ?1",
                rusqlite::params![user_id, next],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Returns true when the raw session token maps to a live session for an
/// active user (honouring expiry).
fn session_valid(db_path: &Path, token: &str) -> Result<bool, String> {
    let token_hash = sha256_hex(token);
    let db = open(db_path)?;
    let found: Option<i64> = db
        .conn
        .query_row(
            "SELECT 1
             FROM web_sessions s JOIN web_users u ON u.id = s.user_id
             WHERE s.token_hash = ?1
               AND s.expires_at > datetime('now')
               AND u.status = 'active'",
            [&token_hash],
            |r| r.get(0),
        )
        .ok();
    Ok(found.is_some())
}

/// Delete a session (logout). Best-effort.
pub fn delete_session(db_path: &Path, token: &str) -> Result<(), String> {
    let token_hash = sha256_hex(token);
    let db = open(db_path)?;
    let _ = db.conn.execute(
        "DELETE FROM web_sessions WHERE token_hash = ?1",
        [&token_hash],
    );
    Ok(())
}

fn open(db_path: &Path) -> Result<toku_db::Database, String> {
    toku_db::Database::open_no_migrate(db_path).map_err(|e| e.to_string())
}

// ── Cookie builders ──────────────────────────────────────────────────────────

/// Build the session cookie (HttpOnly, SameSite=Lax).
pub fn session_cookie(value: String, secure: bool) -> Cookie<'static> {
    let mut c = Cookie::new(SESSION_COOKIE, value);
    c.set_http_only(true);
    c.set_same_site(SameSite::Lax);
    c.set_path("/");
    c.set_secure(secure);
    c.set_max_age(time::Duration::hours(SESSION_TTL_HOURS));
    c
}

/// Build an expired session cookie to clear it on logout.
pub fn clear_session_cookie(secure: bool) -> Cookie<'static> {
    let mut c = Cookie::new(SESSION_COOKIE, "");
    c.set_http_only(true);
    c.set_same_site(SameSite::Lax);
    c.set_path("/");
    c.set_secure(secure);
    c.set_max_age(time::Duration::seconds(0));
    c
}

/// Build the CSRF double-submit cookie (HttpOnly, SameSite=Strict).
fn csrf_cookie(value: String, secure: bool) -> Cookie<'static> {
    let mut c = Cookie::new(CSRF_COOKIE, value);
    c.set_http_only(true);
    c.set_same_site(SameSite::Strict);
    c.set_path("/");
    c.set_secure(secure);
    c
}

// ── Middleware ───────────────────────────────────────────────────────────────

/// Gate protected routes: redirect to onboarding/login when unauthenticated.
///
/// Mounted only in hosted mode.
pub async fn require_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let db_path = state.db_path.clone();

    // First-run: no admin yet → force onboarding.
    let has_admin = match run_blocking(move || admin_exists(&db_path)).await {
        Ok(v) => v,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if !has_admin {
        return Redirect::to("/setup").into_response();
    }

    let token = jar.get(SESSION_COOKIE).map(|c| c.value().to_string());
    let Some(token) = token else {
        return Redirect::to("/login").into_response();
    };

    let db_path = state.db_path.clone();
    let valid = match run_blocking(move || session_valid(&db_path, &token)).await {
        Ok(v) => v,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if valid {
        next.run(req).await
    } else {
        Redirect::to("/login").into_response()
    }
}

/// Double-submit CSRF protection for all routes (mounted in hosted mode).
///
/// Ensures a `toku_csrf` cookie exists and injects the token into request
/// extensions so forms can embed it. On unsafe methods the submitted token
/// (urlencoded `csrf_token` field, or `?csrf=` query for multipart) must match
/// the cookie.
pub async fn csrf_protect(
    State(state): State<AppState>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let existing = jar.get(CSRF_COOKIE).map(|c| c.value().to_string());
    let (token, newly_issued) = match existing {
        Some(t) => (t, false),
        None => (generate_token(), true),
    };

    let unsafe_method = !matches!(
        *req.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    );

    if unsafe_method {
        // A request that can't carry our cookie back (newly issued = no cookie
        // was sent) cannot have a valid token.
        if newly_issued {
            return (StatusCode::FORBIDDEN, "missing CSRF token").into_response();
        }
        let (submitted, rebuilt) = match extract_csrf_submission(req).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        req = rebuilt;
        let ok = submitted
            .map(|s| s.as_bytes().ct_eq(token.as_bytes()).into())
            .unwrap_or(false);
        if !ok {
            return (StatusCode::FORBIDDEN, "CSRF validation failed").into_response();
        }
    }

    req.extensions_mut().insert(CsrfToken(token.clone()));
    let mut resp = next.run(req).await;
    if newly_issued {
        let cookie = csrf_cookie(token, state.secure_cookies);
        if let Ok(value) = header::HeaderValue::from_str(&cookie.to_string()) {
            resp.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    resp
}

/// Pull the CSRF token out of an unsafe request without losing the body.
///
/// For `application/x-www-form-urlencoded` the body is buffered, scanned for the
/// `csrf_token` field, and a fresh request is rebuilt so downstream extractors
/// still see the full body. For multipart (file uploads) the token is read from
/// the `?csrf=` query parameter instead, leaving the streamed body untouched.
async fn extract_csrf_submission(req: Request) -> Result<(Option<String>, Request), Response> {
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if content_type.starts_with("application/x-www-form-urlencoded") {
        let (parts, body) = req.into_parts();
        let bytes = match axum::body::to_bytes(body, CSRF_BODY_LIMIT).await {
            Ok(b) => b,
            Err(_) => {
                return Err(
                    (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response()
                );
            }
        };
        let token = form_urlencoded::parse(&bytes)
            .find(|(k, _)| k == CSRF_FIELD)
            .map(|(_, v)| v.into_owned());
        let req = Request::from_parts(parts, axum::body::Body::from(bytes));
        Ok((token, req))
    } else {
        // Multipart or anything else: read from the query string.
        let token = req.uri().query().and_then(|q| {
            form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "csrf")
                .map(|(_, v)| v.into_owned())
        });
        Ok((token, req))
    }
}

/// Run a blocking DB closure on the blocking pool.
async fn run_blocking<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}
