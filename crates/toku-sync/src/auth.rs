//! Authentication middleware and SRP-6a handlers for the Toku sync server.
//!
//! # SRP Parameter Choices
//!
//! - **Protocol**: SRP-6a (RFC 5054) — the `srp` crate (v0.7.0-rc.3, RustCrypto).
//! - **Group**: RFC 5054 Group 14 — 2048-bit MODP (`G_2048`). Smaller groups
//!   (1024-bit, 1536-bit) are deprecated in the `srp` crate as insufficiently
//!   secure. 2048-bit is the current recommended minimum.
//! - **Hash**: SHA-256 — standard, widely deployed, RustCrypto-reviewed.
//! - **Identity** (SRP username): `library_id` — every device in a library shares
//!   one account password, matching the 1Password-style model where the "vault"
//!   password authenticates access to a shared library.
//! - **SRP salt**: 16 bytes (128 bits) of CSPRNG output, hex-encoded. Stored in
//!   `accounts.srp_salt`. Sent to the client during the challenge step so the
//!   client can recompute `x = H(salt || H(identity || ":" || password))`.
//! - **Verifier**: `v = g^x mod N`, up to 256 bytes for G_2048, hex-encoded. Stored
//!   in `accounts.srp_verifier`. The server never stores or transmits the password.
//! - **Server ephemeral** (`b`): 48 bytes (384 bits) of CSPRNG output, single-use,
//!   stored in `srp_challenges` for at most 5 minutes.
//! - **Session tokens**: 256-bit CSPRNG random, SHA-256 hash stored in `sessions`,
//!   24-hour TTL. Used as opaque Bearer tokens for subsequent API calls.
//! - **Rate limiting**: 5 consecutive failed verifications lock the account for 15
//!   minutes (HTTP 423). Successful login resets the counter.

use std::fmt::Write;
use std::path::PathBuf;

use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use srp::ServerG2048;

use crate::db::SyncDatabase;
use crate::error::SyncError;
use crate::models::{
    EnrollRequest, EnrollResponse, SrpChallengeRequest, SrpChallengeResponse, SrpVerifyRequest,
    SrpVerifyResponse,
};

/// Maximum failed login attempts before account lockout.
const MAX_FAILED_ATTEMPTS: i64 = 5;
/// Lockout duration in minutes.
const LOCKOUT_MINUTES: i64 = 15;
/// SRP challenge TTL in seconds (5 minutes).
const CHALLENGE_TTL_SECS: i64 = 300;
/// Session token TTL in hours.
const SESSION_TTL_HOURS: i64 = 24;

/// Authenticated device identity, injected by the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthDevice {
    pub device_id: String,
    pub library_id: String,
}

impl<S: Send + Sync> FromRequestParts<S> for AuthDevice {
    type Rejection = SyncError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthDevice>()
            .cloned()
            .ok_or(SyncError::Unauthorized)
    }
}

/// SHA-256 hash a string and return the hex-encoded digest.
pub fn sha256_hex(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in hash {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Generate a cryptographically random bearer token (256-bit, base64url no-pad).
pub fn generate_token() -> String {
    use base64::Engine;
    use rand::RngExt;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ── Auth middleware ──────────────────────────────────────────────────────────

/// Middleware that validates Bearer tokens and injects [`AuthDevice`].
///
/// Checks the `sessions` table first (SRP-issued, short-lived tokens), then
/// falls back to `devices.auth_token_hash` for backward-compatible passwordless
/// libraries that registered via the old `POST /api/v1/register` path.
pub async fn require_auth(
    State(db_path): State<PathBuf>,
    mut req: Request,
    next: Next,
) -> Result<Response, SyncError> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(SyncError::Unauthorized)?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(SyncError::Unauthorized)?;

    let token_hash = sha256_hex(token);

    let device = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let token_hash = token_hash.clone();
        move || -> Result<AuthDevice, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // 1. Check the sessions table (SRP-issued, short-lived).
            let session = db.conn.query_row(
                "SELECT s.device_id, s.library_id
                 FROM sessions s
                 WHERE s.session_token_hash = ?1
                   AND s.expires_at > datetime('now')",
                [&token_hash],
                |row| {
                    Ok(AuthDevice {
                        device_id: row.get(0)?,
                        library_id: row.get(1)?,
                    })
                },
            );
            if let Ok(device) = session {
                let _ = db.conn.execute(
                    "UPDATE devices SET last_seen = datetime('now') WHERE device_id = ?1",
                    [&device.device_id],
                );
                return Ok(device);
            }

            // 2. Legacy static bearer token (passwordless libraries).
            let device = db
                .conn
                .query_row(
                    "SELECT device_id, library_id
                     FROM devices WHERE auth_token_hash = ?1",
                    [&token_hash],
                    |row| {
                        Ok(AuthDevice {
                            device_id: row.get(0)?,
                            library_id: row.get(1)?,
                        })
                    },
                )
                .map_err(|_| SyncError::Unauthorized)?;

            db.conn.execute(
                "UPDATE devices SET last_seen = datetime('now') WHERE device_id = ?1",
                [&device.device_id],
            )?;

            Ok(device)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    req.extensions_mut().insert(device);
    Ok(next.run(req).await)
}

// ── SRP handlers ─────────────────────────────────────────────────────────────

/// `POST /api/v1/auth/enroll`
///
/// Register the first device for a new library using SRP-6a. The client
/// computes the SRP verifier locally (`v = g^x mod N` where
/// `x = H(salt || H(library_id || ":" || password))`) and uploads only the
/// verifier and salt — the server never sees the password.
///
/// Fails if the library already has an SRP account to prevent takeover attacks.
pub async fn srp_enroll(
    State(db_path): State<PathBuf>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, SyncError> {
    if req.library_id.is_empty() {
        return Err(SyncError::BadRequest("library_id is required".into()));
    }
    if req.device_name.is_empty() {
        return Err(SyncError::BadRequest("device_name is required".into()));
    }
    // Validate hex encoding before hitting the DB.
    hex::decode(&req.srp_salt)
        .map_err(|_| SyncError::BadRequest("srp_salt must be hex-encoded".into()))?;
    hex::decode(&req.srp_verifier)
        .map_err(|_| SyncError::BadRequest("srp_verifier must be hex-encoded".into()))?;

    let device_id = uuid::Uuid::now_v7().to_string();

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = req.library_id.clone();
        let device_name = req.device_name.clone();
        let device_id = device_id.clone();
        let srp_salt = req.srp_salt.clone();
        let srp_verifier = req.srp_verifier.clone();
        let encryption_salt = req.encryption_salt.clone();
        move || -> Result<EnrollResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Reject re-enrollment: if this library already has SRP credentials, the
            // caller must authenticate first, then call POST /api/v1/register.
            let already_enrolled: bool = db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE library_id = ?1",
                    [&library_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if already_enrolled {
                return Err(SyncError::Forbidden(
                    "library already has SRP credentials; \
                     authenticate via /auth/challenge + /auth/verify, \
                     then use POST /api/v1/register to add this device"
                        .into(),
                ));
            }

            // Auto-create the library record.
            db.conn.execute(
                "INSERT OR IGNORE INTO libraries (id, created_at) VALUES (?1, datetime('now'))",
                [&library_id],
            )?;

            // Optionally store the encryption salt (first writer wins, same as register).
            if let Some(ref enc_salt) = encryption_salt {
                db.conn.execute(
                    "UPDATE libraries SET salt = ?1 WHERE id = ?2 AND salt IS NULL",
                    rusqlite::params![enc_salt, library_id],
                )?;
            }

            // Store SRP credentials — verifier and salt, never the password.
            db.conn.execute(
                "INSERT INTO accounts (library_id, srp_salt, srp_verifier)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![library_id, srp_salt, srp_verifier],
            )?;

            // Create the device record (empty auth_token_hash; SRP libraries use sessions).
            db.conn.execute(
                "INSERT INTO devices (device_id, library_id, device_name, auth_token_hash, created_at)
                 VALUES (?1, ?2, ?3, '', datetime('now'))",
                rusqlite::params![device_id, library_id, device_name],
            )?;

            Ok(EnrollResponse {
                device_id,
                library_id,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// `POST /api/v1/auth/challenge`
///
/// Start an SRP-6a login. The client sends its public ephemeral A
/// (`g^a mod N`); the server looks up the verifier, generates a server
/// ephemeral B, stores the challenge (single-use, 5 min TTL), and returns B
/// together with the SRP salt so the client can derive `x`.
///
/// The client must call `POST /api/v1/auth/verify` with the same
/// `challenge_id` within 5 minutes.
pub async fn srp_challenge(
    State(db_path): State<PathBuf>,
    Json(req): Json<SrpChallengeRequest>,
) -> Result<Json<SrpChallengeResponse>, SyncError> {
    if req.library_id.is_empty() {
        return Err(SyncError::BadRequest("library_id is required".into()));
    }
    // Validate A before touching the DB.
    let a_pub_bytes = hex::decode(&req.client_public_a)
        .map_err(|_| SyncError::BadRequest("client_public_a must be hex-encoded".into()))?;
    if a_pub_bytes.is_empty() || a_pub_bytes.iter().all(|&b| b == 0) {
        return Err(SyncError::BadRequest(
            "client_public_a must be a non-zero group element".into(),
        ));
    }

    let challenge_id = uuid::Uuid::now_v7().to_string();
    let client_public_a_hex = req.client_public_a.clone();

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = req.library_id.clone();
        let challenge_id = challenge_id.clone();
        move || -> Result<SrpChallengeResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Load the SRP account; reject unknown or locked libraries.
            let (srp_salt_hex, srp_verifier_hex, locked_until): (String, String, Option<String>) =
                db.conn
                    .query_row(
                        "SELECT srp_salt, srp_verifier, locked_until
                         FROM accounts WHERE library_id = ?1",
                        [&library_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .map_err(|_| {
                        SyncError::NotFound(format!(
                            "no SRP account for library '{library_id}'; \
                             enroll first via POST /api/v1/auth/enroll"
                        ))
                    })?;

            if let Some(until) = locked_until {
                let still_locked: bool = db
                    .conn
                    .query_row("SELECT ?1 > datetime('now')", [&until], |row| row.get(0))
                    .unwrap_or(false);
                if still_locked {
                    return Err(SyncError::AccountLocked { until });
                }
            }

            let verifier_bytes = hex::decode(&srp_verifier_hex)
                .map_err(|_| SyncError::Internal("stored verifier is corrupt".into()))?;

            // Generate a fresh 384-bit server ephemeral b.
            let mut b = [0u8; 48];
            rand::RngExt::fill(&mut rand::rng(), &mut b);

            // Compute B = k*v + g^b mod N.
            let server = ServerG2048::<Sha256>::new();
            let b_pub_bytes = server.compute_public_ephemeral(&b, &verifier_bytes);

            let server_ephemeral_hex = hex::encode(b);
            let b_pub_hex = hex::encode(&b_pub_bytes);

            // Persist the challenge row (single-use).
            db.conn.execute(
                "INSERT INTO srp_challenges
                 (challenge_id, library_id, server_ephemeral_secret, client_public_a, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![
                    challenge_id,
                    library_id,
                    server_ephemeral_hex,
                    client_public_a_hex
                ],
            )?;

            // Prune stale challenges (best-effort, non-fatal).
            let _ = db.conn.execute(
                "DELETE FROM srp_challenges
                 WHERE created_at < datetime('now', ?1)",
                [format!("-{CHALLENGE_TTL_SECS} seconds")],
            );

            Ok(SrpChallengeResponse {
                challenge_id,
                server_public_b: b_pub_hex,
                srp_salt: srp_salt_hex,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// `POST /api/v1/auth/verify`
///
/// Complete an SRP-6a login. The client sends M1 (proof of knowledge of the
/// password). The server verifies M1 — which cryptographically guarantees the
/// client knows the verifier preimage — and issues a 24-hour session token if
/// the proof is correct.
///
/// On success, the response also contains M2 (server proof) which the client
/// MUST verify before trusting the session token.
///
/// On failure the `failed_attempts` counter increments; after 5 consecutive
/// failures the account is locked for 15 minutes (HTTP 423).
pub async fn srp_verify(
    State(db_path): State<PathBuf>,
    Json(req): Json<SrpVerifyRequest>,
) -> Result<Json<SrpVerifyResponse>, SyncError> {
    if req.challenge_id.is_empty() {
        return Err(SyncError::BadRequest("challenge_id is required".into()));
    }
    let m1_bytes = hex::decode(&req.client_proof_m1)
        .map_err(|_| SyncError::BadRequest("client_proof_m1 must be hex-encoded".into()))?;
    if m1_bytes.is_empty() {
        return Err(SyncError::BadRequest("client_proof_m1 is empty".into()));
    }

    // Generate the session token here so we can move it into the blocking task.
    let session_token = generate_token();
    let token_hash = sha256_hex(&session_token);

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let challenge_id = req.challenge_id.clone();
        let m1_bytes = m1_bytes.clone();
        let session_token = session_token.clone();
        let token_hash = token_hash.clone();
        move || -> Result<SrpVerifyResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Load and validate the challenge (will be deleted on success or expiry).
            let (library_id, server_ephemeral_hex, client_public_a_hex, created_at): (
                String,
                String,
                String,
                String,
            ) = db
                .conn
                .query_row(
                    "SELECT library_id, server_ephemeral_secret, client_public_a, created_at
                     FROM srp_challenges WHERE challenge_id = ?1",
                    [&challenge_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|_| SyncError::BadRequest("challenge not found or already used".into()))?;

            // Enforce 5-minute TTL.
            let expired: bool = db
                .conn
                .query_row(
                    "SELECT ?1 < datetime('now', ?2)",
                    rusqlite::params![created_at, format!("-{CHALLENGE_TTL_SECS} seconds")],
                    |row| row.get(0),
                )
                .unwrap_or(true);
            if expired {
                let _ = db.conn.execute(
                    "DELETE FROM srp_challenges WHERE challenge_id = ?1",
                    [&challenge_id],
                );
                return Err(SyncError::BadRequest(
                    "challenge has expired; request a new one".into(),
                ));
            }

            // Load SRP account with rate-limiting state.
            let (srp_salt_hex, srp_verifier_hex, failed_attempts, locked_until): (
                String,
                String,
                i64,
                Option<String>,
            ) = db
                .conn
                .query_row(
                    "SELECT srp_salt, srp_verifier, failed_attempts, locked_until
                     FROM accounts WHERE library_id = ?1",
                    [&library_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|_| SyncError::Internal("SRP account disappeared".into()))?;

            // Re-check lockout (may have been set by a concurrent verify).
            if let Some(ref until) = locked_until {
                let still_locked: bool = db
                    .conn
                    .query_row("SELECT ?1 > datetime('now')", [until], |row| row.get(0))
                    .unwrap_or(false);
                if still_locked {
                    return Err(SyncError::AccountLocked {
                        until: until.clone(),
                    });
                }
            }

            // Decode all the blobs.
            let salt_bytes = hex::decode(&srp_salt_hex)
                .map_err(|_| SyncError::Internal("stored salt is corrupt".into()))?;
            let verifier_bytes = hex::decode(&srp_verifier_hex)
                .map_err(|_| SyncError::Internal("stored verifier is corrupt".into()))?;
            let b_bytes = hex::decode(&server_ephemeral_hex)
                .map_err(|_| SyncError::Internal("stored server ephemeral is corrupt".into()))?;
            let a_pub_bytes = hex::decode(&client_public_a_hex)
                .map_err(|_| SyncError::Internal("stored client_public_a is corrupt".into()))?;

            // Always delete the challenge to enforce single-use semantics.
            let _ = db.conn.execute(
                "DELETE FROM srp_challenges WHERE challenge_id = ?1",
                [&challenge_id],
            );

            // Run SRP verification. `process_reply` reconstructs the expected M1 from
            // all public inputs; `verify_client` does a constant-time comparison.
            let server = ServerG2048::<Sha256>::new();
            let server_verifier = match server.process_reply(
                library_id.as_bytes(),
                &salt_bytes,
                &b_bytes,
                &verifier_bytes,
                &a_pub_bytes,
            ) {
                Ok(v) => v,
                Err(_) => {
                    record_failure(&db, &library_id, failed_attempts)?;
                    return Err(SyncError::Unauthorized);
                }
            };

            match server_verifier.verify_client(&m1_bytes) {
                Ok(_) => {}
                Err(_) => {
                    return record_failure_and_err(&db, &library_id, failed_attempts);
                }
            }

            let m2_hex = hex::encode(server_verifier.proof());

            // Successful login: reset rate-limiting.
            let _ = db.conn.execute(
                "UPDATE accounts SET failed_attempts = 0, locked_until = NULL
                 WHERE library_id = ?1",
                [&library_id],
            );

            // Resolve the device_id for this library.
            // For subsequent-device logins the most-recently-enrolled device may not
            // be the caller; the caller passes `device_name` during register. For
            // enroll (first device) there is exactly one device.
            let device_id: String = db
                .conn
                .query_row(
                    "SELECT device_id FROM devices
                     WHERE library_id = ?1 ORDER BY created_at DESC LIMIT 1",
                    [&library_id],
                    |row| row.get(0),
                )
                .map_err(|_| SyncError::Internal("no device found for library".into()))?;

            // Compute expiry time.
            let expires_at: String = db
                .conn
                .query_row(
                    "SELECT datetime('now', ?1)",
                    [format!("+{SESSION_TTL_HOURS} hours")],
                    |row| row.get(0),
                )
                .map_err(|e| SyncError::Internal(format!("datetime query failed: {e}")))?;

            // Issue the session token.
            db.conn.execute(
                "INSERT INTO sessions
                 (session_token_hash, device_id, library_id, expires_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![token_hash, device_id, library_id, expires_at],
            )?;

            Ok(SrpVerifyResponse {
                session_token,
                server_proof_m2: m2_hex,
                expires_at,
                device_id,
                library_id,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

// ── Rate-limiting helpers ────────────────────────────────────────────────────

/// Increment the failed-attempt counter; lock the account if the threshold is hit.
fn record_failure(
    db: &SyncDatabase,
    library_id: &str,
    current_attempts: i64,
) -> Result<(), SyncError> {
    let new_count = current_attempts + 1;
    if new_count >= MAX_FAILED_ATTEMPTS {
        db.conn.execute(
            "UPDATE accounts SET failed_attempts = ?1,
             locked_until = datetime('now', ?2)
             WHERE library_id = ?3",
            rusqlite::params![new_count, format!("+{LOCKOUT_MINUTES} minutes"), library_id],
        )?;
    } else {
        db.conn.execute(
            "UPDATE accounts SET failed_attempts = ?1 WHERE library_id = ?2",
            rusqlite::params![new_count, library_id],
        )?;
    }
    Ok(())
}

/// Increment the counter and return the appropriate error.
fn record_failure_and_err(
    db: &SyncDatabase,
    library_id: &str,
    current_attempts: i64,
) -> Result<SrpVerifyResponse, SyncError> {
    let new_count = current_attempts + 1;
    if new_count >= MAX_FAILED_ATTEMPTS {
        db.conn
            .execute(
                "UPDATE accounts SET failed_attempts = ?1,
                 locked_until = datetime('now', ?2)
                 WHERE library_id = ?3",
                rusqlite::params![new_count, format!("+{LOCKOUT_MINUTES} minutes"), library_id],
            )
            .ok();
        let until: String = db
            .conn
            .query_row(
                "SELECT locked_until FROM accounts WHERE library_id = ?1",
                [library_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "unknown".into());
        Err(SyncError::AccountLocked { until })
    } else {
        db.conn
            .execute(
                "UPDATE accounts SET failed_attempts = ?1 WHERE library_id = ?2",
                rusqlite::params![new_count, library_id],
            )
            .ok();
        Err(SyncError::Unauthorized)
    }
}
