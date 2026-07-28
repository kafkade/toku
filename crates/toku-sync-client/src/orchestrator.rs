//! High-level sync orchestration shared by the CLI and FFI frontends.
//!
//! These functions encapsulate the full push/pull/init/status flows that previously
//! lived inline in the `toku-cli` binary. They take a data directory, perform the
//! network + database work, and return structured outcomes. They never print and never
//! prompt — callers (CLI, FFI) are responsible for I/O and presentation.

use std::path::Path;

use anyhow::Context;
use base64::Engine;
use serde::Serialize;
use toku_core::SyncKey;
use toku_db::{
    ConflictKeep, Database, MergeEngine, SnapshotRepository, SyncConflict, SyncRepository,
};

use crate::client::{DeviceInfo, SyncClient, WireOp};
use crate::token_store::TokenStore;
use crate::wire;

/// Outcome of [`init`].
#[derive(Debug, Clone, Serialize)]
pub struct InitOutcome {
    pub device_id: String,
    pub library_id: String,
    pub device_name: String,
    pub server: String,
    pub encryption: bool,
}

/// Outcome of [`push`].
#[derive(Debug, Clone, Serialize)]
pub struct PushOutcome {
    /// Total number of local ops that were pending before the push.
    pub pushed: usize,
    pub accepted: usize,
    pub duplicates: usize,
    pub cursor: Option<String>,
    /// True when there was nothing to push.
    pub up_to_date: bool,
}

/// Outcome of [`pull`].
#[derive(Debug, Clone, Serialize)]
pub struct PullOutcome {
    /// Number of remote ops fetched from the server.
    pub pulled: usize,
    /// Number of ops that were successfully applied to local state.
    pub applied: usize,
    /// Number of merge conflicts recorded for user review during this pull.
    pub conflicts: usize,
    pub cursor: Option<String>,
}

/// Outcome of [`status`].
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    pub enabled: bool,
    pub server: String,
    pub device_id: String,
    pub device_name: String,
    pub library_id: String,
    pub encryption: bool,
    pub pending_ops: usize,
    pub push_cursor: Option<String>,
    pub pull_cursor: Option<String>,
    pub device_count: usize,
    pub unresolved_conflicts: usize,
}

/// Outcome of [`bootstrap`].
#[derive(Debug, Clone, Serialize)]
pub struct BootstrapOutcome {
    /// Whether a server snapshot was found and applied locally.
    pub snapshot_applied: bool,
    /// Number of books loaded from the snapshot.
    pub snapshot_books: usize,
    /// Number of ops pulled after the snapshot (post-snapshot history).
    pub pulled: usize,
    /// Number of pulled ops applied to local state.
    pub applied: usize,
}

/// Report of a first-opt-in op-backfill (#199, ADR-013 D2): how many pre-existing
/// rows were staged as ops and how they fared on push.
#[derive(Debug, Clone, Serialize)]
pub struct BackfillReport {
    pub books: usize,
    pub sessions: usize,
    pub progress: usize,
    pub tags: usize,
    /// Total ops synthesized across all entity types.
    pub ops_total: usize,
    /// Ops accepted by the server on the backfill push.
    pub pushed: usize,
}

/// Outcome of [`signup`].
#[derive(Debug, Clone, Serialize)]
pub struct SignupOutcome {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub device_id: String,
    pub library_id: String,
    pub device_name: String,
    pub server: String,
    /// Device enrollment status (`active` or `pending`).
    pub device_status: String,
    /// The formatted Secret Key — surfaced **once** so the caller can render an
    /// Emergency Kit. Never persisted to disk by the orchestrator.
    pub secret_key: String,
    /// First-opt-in backfill of pre-existing local state (D2). Always present
    /// for signup since the first device always creates a fresh library.
    pub backfill: BackfillReport,
}

/// Outcome of [`login`].
#[derive(Debug, Clone, Serialize)]
pub struct LoginOutcome {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub server: String,
    /// Whether the leaf data key was unwrapped and stored (requires the #143
    /// `GET /api/v1/account/keys` endpoint).
    pub data_key_unlocked: bool,
    /// Present when this login triggered a deferred new-device bootstrap — i.e.
    /// the first login after an approval-pending device was approved (D3).
    pub bootstrap: Option<BootstrapOutcome>,
}

/// Outcome of [`enroll`].
#[derive(Debug, Clone, Serialize)]
pub struct EnrollOutcome {
    pub user_id: String,
    pub email: String,
    pub device_id: String,
    pub library_id: String,
    pub device_name: String,
    pub server: String,
    /// `active` (synced immediately) or `pending` (awaiting approval).
    pub device_status: String,
    /// First-opt-in backfill of pre-existing local state (D2). Present only when
    /// this enroll created a **fresh** library from a device that already held
    /// local data; `None` when joining an existing library.
    pub backfill: Option<BackfillReport>,
    /// New-device bootstrap result (D3). Present when an active device session
    /// was minted and bootstrap ran; `None` for approval-pending devices (whose
    /// bootstrap is deferred to the first post-approval login).
    pub bootstrap: Option<BootstrapOutcome>,
}

/// Outcome of [`migrate`].
#[derive(Debug, Clone, Serialize)]
pub struct MigrateOutcome {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub server: String,
    pub library_id: String,
    pub device_id: String,
    /// Relay libraries adopted under the new admin account (#126).
    pub adopted_libraries: i64,
    /// Relay devices adopted under the new admin account (#126).
    pub adopted_devices: i64,
    /// Ops re-encrypted client-side under the fresh data key.
    pub ops_reencrypted: usize,
    /// Ops the server replaced during rekey.
    pub ops_replaced: usize,
    /// True when ops were encrypted under the legacy single passphrase; false
    /// when previously-plaintext ops were encrypted for the first time.
    pub had_encryption: bool,
    /// The formatted Secret Key — surfaced **once** for the Emergency Kit.
    pub secret_key: String,
}

fn build_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")
}

fn open_db(data_dir: &Path) -> anyhow::Result<Database> {
    Database::open_default(&data_dir.join("toku.db")).context("failed to open database")
}

fn require_sync(config: &toku_core::TokuConfig) -> anyhow::Result<&toku_core::SyncConfig> {
    config.sync.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "sync is not configured. Run `toku sync signup` (new account) or \
             `toku sync enroll` (existing account) first."
        )
    })
}

fn require_token(token_store: &TokenStore, server: &str) -> anyhow::Result<String> {
    token_store.load(server)?.ok_or_else(|| {
        anyhow::anyhow!("no auth token found for {server}. Run `toku sync login` first.")
    })
}

/// Load the client-side encryption key when encryption is enabled for this
/// library; returns `None` when encryption is disabled.
fn load_encryption_key(
    token_store: &TokenStore,
    server: &str,
    sync_config: &toku_core::SyncConfig,
) -> anyhow::Result<Option<SyncKey>> {
    if !sync_config.encryption {
        return Ok(None);
    }
    let bytes = token_store.load_sync_key(server)?.ok_or_else(|| {
        anyhow::anyhow!("encryption is enabled but no sync key was found for {server}")
    })?;
    let key = SyncKey::from_exported_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("stored sync key is invalid: {e}"))?;
    Ok(Some(key))
}

/// Initialize sync: register the device with the server, persist the auth token and
/// sync config, and enable **mandatory** client-side encryption from the passphrase.
///
/// Hosted/sync mode is zero-knowledge (ADR-010, issue #121): a passphrase is
/// **required**. It drives SRP-6a enrollment/login (so the server never sees the
/// password) and derives the client-side AES data key used to encrypt every op.
///
/// Calling without a passphrase returns an error — the previous passwordless,
/// plaintext opt-out has been removed. Local-only single-device usage never
/// uploads and so needs neither sync nor a passphrase.
pub fn init(
    data_dir: &Path,
    server: &str,
    library_id: Option<String>,
    device_name: Option<String>,
    passphrase: Option<&str>,
) -> anyhow::Result<InitOutcome> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);

    let library_id = library_id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let device_name = device_name.unwrap_or_else(default_device_name);

    let client = SyncClient::new(server)?;

    match passphrase {
        Some(pass) if !pass.is_empty() => {
            // ── SRP-6a path ──────────────────────────────────────────────
            //
            // First device:  enroll (creates SRP account) → challenge/verify
            // Second device: skip enroll → challenge/verify → register (bearer)
            //
            // In both cases, the passphrase never leaves this machine.

            use rand::RngExt;
            use sha2::Sha256;
            use srp::ClientG2048;

            let srp_client = ClientG2048::<Sha256>::new();

            // ── Try enrollment (first device only) ───────────────────────
            // Compute verifier locally; upload only v and salt.
            // Also generate the encryption salt and submit it at enrollment so the
            // server stores it — device B will retrieve it via `get_salt`.
            let first_device_resp: Option<crate::client::EnrollResponse> = {
                let mut srp_salt = [0u8; 16];
                rand::rng().fill(&mut srp_salt);
                let srp_salt_hex = hex::encode(srp_salt);
                // Library/passphrase path is single-secret (no Secret Key); route
                // through the same domain-separated derivation for consistency.
                let verifier_input = toku_core::srp_verifier_input(None, pass);
                let verifier_bytes =
                    srp_client.compute_verifier(library_id.as_bytes(), &verifier_input, &srp_salt);
                let srp_verifier_hex = hex::encode(&verifier_bytes);

                let enc_salt_raw = toku_core::SyncKey::generate_salt()?;
                let enc_salt_b64 = base64::engine::general_purpose::STANDARD.encode(enc_salt_raw);

                match rt.block_on(client.enroll(
                    &library_id,
                    &device_name,
                    &srp_salt_hex,
                    &srp_verifier_hex,
                    Some(&enc_salt_b64),
                )) {
                    Ok(resp) => Some(resp),
                    Err(e) if e.to_string().contains("already has SRP credentials") => None,
                    Err(e) => return Err(e),
                }
            };

            // ── SRP login — works for both first and subsequent devices ──
            let mut a = [0u8; 48];
            rand::rng().fill(&mut a);
            let a_pub_bytes = srp_client.compute_public_ephemeral(&a);
            let a_pub_hex = hex::encode(&a_pub_bytes);

            let challenge_resp = rt.block_on(client.srp_challenge(&library_id, &a_pub_hex))?;

            let b_pub_bytes = hex::decode(&challenge_resp.server_public_b)
                .context("server returned invalid hex for server_public_b")?;
            let server_srp_salt_bytes = hex::decode(&challenge_resp.srp_salt)
                .context("server returned invalid hex for srp_salt")?;

            let client_verifier = srp_client
                .process_reply(
                    &a,
                    library_id.as_bytes(),
                    &toku_core::srp_verifier_input(None, pass),
                    &server_srp_salt_bytes,
                    &b_pub_bytes,
                )
                .map_err(|e| anyhow::anyhow!("SRP client processing failed: {e:?}"))?;

            let m1_hex = hex::encode(client_verifier.proof());
            let verify_resp =
                rt.block_on(client.srp_verify(&challenge_resp.challenge_id, &m1_hex))?;

            // Verify server proof M2 — confirms server knows the verifier.
            let m2_bytes = hex::decode(&verify_resp.server_proof_m2)
                .context("server returned invalid hex for server_proof_m2")?;
            client_verifier
                .verify_server(&m2_bytes)
                .map_err(|e| anyhow::anyhow!("SRP server proof verification failed: {e:?}"))?;

            token_store
                .store_session(server, &verify_resp.session_token, &verify_resp.expires_at)
                .context("failed to store SRP session token")?;

            // ── Device ID + library ID ───────────────────────────────────
            let (device_id, library_id_out) = if let Some(ref er) = first_device_resp {
                // First device: library + device already created by enroll.
                (er.device_id.clone(), er.library_id.clone())
            } else {
                // Second device: register this device using the fresh session token.
                let reg_resp = rt.block_on(client.register(
                    &library_id,
                    &device_name,
                    None,
                    Some(&verify_resp.session_token),
                ))?;
                (reg_resp.device_id, reg_resp.library_id)
            };

            // ── Encryption key ───────────────────────────────────────────
            // Fetch the authoritative encryption salt from the server; fall back
            // to the candidate we submitted during enroll (first-writer-wins).
            let enc_salt_b64 = match rt
                .block_on(client.get_salt(&verify_resp.session_token))?
                .salt
            {
                Some(salt) => salt,
                None => {
                    // Should only happen if server lost the salt — regenerate safely.
                    let raw = toku_core::SyncKey::generate_salt()?;
                    base64::engine::general_purpose::STANDARD.encode(raw)
                }
            };
            let enc_salt_bytes = base64::engine::general_purpose::STANDARD
                .decode(&enc_salt_b64)
                .context("invalid base64 encryption salt from server")?;
            let enc_salt: [u8; 16] = enc_salt_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("encryption salt must be 16 bytes"))?;
            let key = toku_core::SyncKey::derive(pass, &enc_salt)
                .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
            token_store
                .store_sync_key(server, key.as_exported_bytes())
                .context("failed to store sync key")?;

            // Record the device in the local DB.
            let db = open_db(data_dir)?;
            let sync_repo = SyncRepository::new(&db);
            let server_device_id = device_id
                .parse::<uuid::Uuid>()
                .context("server returned an invalid device_id")?;
            sync_repo.get_or_create_device_with_id(server_device_id, &device_name)?;

            let mut config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
            config.sync = Some(toku_core::SyncConfig {
                server: server.to_string(),
                library_id: library_id_out.clone(),
                device_id: device_id.clone(),
                device_name: device_name.clone(),
                encryption: true,
            });
            config
                .save(data_dir)
                .map_err(|e| anyhow::anyhow!("failed to save config: {e}"))?;

            Ok(InitOutcome {
                device_id,
                library_id: library_id_out,
                device_name,
                server: server.to_string(),
                encryption: true,
            })
        }

        _ => {
            // ── Plaintext opt-out removed (issue #121) ───────────────────
            //
            // Hosted/sync mode now mandates client-side E2E encryption
            // (zero-knowledge). A passwordless library would upload plaintext
            // ops, which the server rejects, so we refuse up front with an
            // actionable error instead of registering an unusable device.
            Err(anyhow::anyhow!(
                "hosted sync requires client-side encryption: a passphrase is mandatory.\n\
                 Run `toku sync signup` (new account) or `toku sync enroll` (existing account) \
                 to set up encryption.\n\
                 Local-only, single-device usage needs no sync and no passphrase."
            ))
        }
    }
}

/// Push all locally pending ops to the configured sync server.
pub fn push(data_dir: &Path) -> anyhow::Result<PushOutcome> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let sync_config = require_sync(&config)?;
    let server = &sync_config.server;
    let token = require_token(&token_store, server)?;

    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    let client = SyncClient::new(server)?;

    let unpushed = sync_repo.get_unpushed_ops()?;
    if unpushed.is_empty() {
        return Ok(PushOutcome {
            pushed: 0,
            accepted: 0,
            duplicates: 0,
            cursor: None,
            up_to_date: true,
        });
    }

    let total = unpushed.len();
    // Zero-knowledge: hosted mode mandates client-side encryption. Refuse to
    // push if no key is configured rather than uploading plaintext.
    let key = load_encryption_key(&token_store, server, sync_config)?.ok_or_else(|| {
        anyhow::anyhow!(
            "hosted sync requires client-side encryption but no key is configured.\n\
             Run `toku sync login` to unlock your library key with your password and Secret Key, \
             or `toku sync enroll` to enroll this device into your account."
        )
    })?;
    let wire_ops: Vec<WireOp> = unpushed
        .iter()
        .map(|op| {
            let mut encrypted = op.clone();
            encrypted
                .encrypt(&key)
                .map_err(|e| anyhow::anyhow!("failed to encrypt op {}: {e}", op.op_id))?;
            Ok(wire::to_wire(&encrypted))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut total_accepted = 0usize;
    let mut total_duplicates = 0usize;
    let mut last_cursor = None;

    for chunk in wire_ops.chunks(1000) {
        let result = rt.block_on(client.push_ops(&token, chunk))?;
        total_accepted += result.accepted;
        total_duplicates += result.duplicates;
        if result.new_cursor.is_some() {
            last_cursor = result.new_cursor;
        }
    }

    let op_ids: Vec<uuid::Uuid> = unpushed.iter().map(|op| op.op_id).collect();
    sync_repo.mark_ops_pushed(&op_ids)?;

    if let Some(ref cursor) = last_cursor {
        sync_repo.set_cursor("push_cursor", cursor)?;
    }

    Ok(PushOutcome {
        pushed: total,
        accepted: total_accepted,
        duplicates: total_duplicates,
        cursor: last_cursor,
        up_to_date: false,
    })
}

/// Pull remote ops from the configured sync server and apply them locally.
pub fn pull(data_dir: &Path) -> anyhow::Result<PullOutcome> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let sync_config = require_sync(&config)?;
    let server = &sync_config.server;
    let token = require_token(&token_store, server)?;

    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    let merge_engine = MergeEngine::new(&db);
    let client = SyncClient::new(server)?;
    let key = load_encryption_key(&token_store, server, sync_config)?;

    let mut cursor = sync_repo.get_cursor("pull_cursor")?;
    let mut total_pulled = 0usize;
    let mut total_applied = 0usize;
    let mut total_conflicts = 0usize;

    loop {
        let result = rt.block_on(client.pull_ops(&token, cursor.as_deref()))?;
        if result.ops.is_empty() {
            break;
        }
        for wire_op in &result.ops {
            let mut sync_op = wire::from_wire(wire_op).context("failed to parse remote op")?;

            // Decrypt before staging/applying when the op carries an encrypted
            // envelope. An encrypted op with no local key is a misconfiguration.
            if sync_op.encrypted.is_some() {
                let key = key.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "received an encrypted op ({}) but encryption is not configured locally",
                        sync_op.op_id
                    )
                })?;
                sync_op
                    .decrypt(key)
                    .with_context(|| format!("failed to decrypt remote op {}", sync_op.op_id))?;
            }

            // Stage the op in the local op-log, then materialize it into the
            // entity tables via the entity-specific merge engine.
            sync_repo.insert_remote_op(&sync_op)?;
            let outcome = merge_engine
                .apply_op(&sync_op)
                .with_context(|| format!("failed to apply remote op {}", sync_op.op_id))?;
            if outcome.was_applied() {
                total_applied += 1;
            }
            total_conflicts += outcome.conflicts().len();
        }
        total_pulled += result.ops.len();
        if let Some(new_cursor) = result.cursor {
            sync_repo.set_cursor("pull_cursor", &new_cursor)?;
            cursor = Some(new_cursor);
        }
        if !result.has_more {
            break;
        }
    }

    Ok(PullOutcome {
        pulled: total_pulled,
        applied: total_applied,
        conflicts: total_conflicts,
        cursor,
    })
}

/// Bootstrap a freshly-registered device: download the latest server snapshot
/// (if one exists from op-log compaction), apply it locally, then pull any ops
/// the server still retains. Used for new-device provisioning.
///
/// When no snapshot exists the server still holds the full op log, so this is
/// equivalent to a plain [`pull`].
///
/// When `reset_cursor` is set the local pull cursor is cleared first, forcing a
/// full re-download of the snapshot and a re-pull from op #1 — the recovery path
/// for a device whose local state is suspect (`toku sync bootstrap --reset-cursor`).
pub fn bootstrap(data_dir: &Path, reset_cursor: bool) -> anyhow::Result<BootstrapOutcome> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let sync_config = require_sync(&config)?;
    let server = &sync_config.server;
    let token = require_token(&token_store, server)?;
    let client = SyncClient::new(server)?;

    if reset_cursor {
        let db = open_db(data_dir)?;
        SyncRepository::new(&db)
            .clear_cursor("pull_cursor")
            .context("failed to reset pull cursor")?;
    }

    let mut snapshot_applied = false;
    let mut snapshot_books = 0usize;

    if let Some(snap) = rt.block_on(client.download_snapshot(&token))? {
        // Zero-knowledge: snapshots are stored as ciphertext. Decrypt with the
        // library data key before applying.
        let key = load_encryption_key(&token_store, server, sync_config)?.ok_or_else(|| {
            anyhow::anyhow!(
                "downloaded an encrypted snapshot but no key is configured.\n\
                 Run `toku sync login` to unlock your library key with your password and \
                 Secret Key, or `toku sync enroll` to enroll this device into your account."
            )
        })?;
        let envelope: toku_core::EncryptedEnvelope = serde_json::from_str(&snap.snapshot_json)
            .context("snapshot is not an encrypted envelope")?;
        let snapshot_json = toku_core::decrypt_snapshot(&key, &envelope)
            .map_err(|e| anyhow::anyhow!("failed to decrypt snapshot: {e}"))?;
        let snapshot: toku_core::sync::LibrarySnapshot =
            serde_json::from_str(&snapshot_json).context("invalid snapshot JSON")?;
        let db = open_db(data_dir)?;
        let snapshot_repo = SnapshotRepository::new(&db);
        let result = snapshot_repo
            .apply_snapshot(&snapshot)
            .context("failed to apply snapshot")?;
        snapshot_applied = true;
        snapshot_books = result.books;
    }

    // Pull whatever the server still retains (post-snapshot ops, or the full
    // op log if no snapshot existed) and materialize it on top of the snapshot.
    let pulled = pull(data_dir)?;

    // Record that this device has completed a bootstrap so a later routine
    // `login` does not re-run the deferred new-device restore (D3).
    mark_bootstrapped(data_dir)?;

    Ok(BootstrapOutcome {
        snapshot_applied,
        snapshot_books,
        pulled: pulled.pulled,
        applied: pulled.applied,
    })
}

/// number of registered devices (best-effort network call).
pub fn status(data_dir: &Path) -> anyhow::Result<StatusOutcome> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let sync_config = require_sync(&config)?;
    let server = sync_config.server.clone();

    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);

    let pending = sync_repo.count_unpushed_ops()?;
    let push_cursor = sync_repo.get_cursor("push_cursor")?;
    let pull_cursor = sync_repo.get_cursor("pull_cursor")?;
    let unresolved_conflicts = sync_repo.count_unresolved_conflicts()? as usize;

    let device_count = token_store
        .load(&server)?
        .and_then(|token| {
            let client = SyncClient::new(&server).ok()?;
            rt.block_on(client.list_devices(&token))
                .ok()
                .map(|d| d.len())
        })
        .unwrap_or(0);

    Ok(StatusOutcome {
        enabled: true,
        server,
        device_id: sync_config.device_id.clone(),
        device_name: sync_config.device_name.clone(),
        library_id: sync_config.library_id.clone(),
        encryption: sync_config.encryption,
        pending_ops: pending as usize,
        push_cursor,
        pull_cursor,
        device_count,
        unresolved_conflicts,
    })
}

/// List the devices registered to this library on the sync server.
pub fn devices(data_dir: &Path) -> anyhow::Result<Vec<DeviceInfo>> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let sync_config = require_sync(&config)?;
    let server = &sync_config.server;
    let token = require_token(&token_store, server)?;

    let client = SyncClient::new(server)?;
    let devices = rt.block_on(client.list_devices(&token))?;
    Ok(devices)
}

/// List all unresolved sync conflicts awaiting user review.
pub fn conflicts(data_dir: &Path) -> anyhow::Result<Vec<SyncConflict>> {
    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    Ok(sync_repo.list_unresolved_conflicts()?)
}

/// Fetch a single conflict by id.
pub fn conflict(data_dir: &Path, id: &str) -> anyhow::Result<Option<SyncConflict>> {
    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    Ok(sync_repo.get_conflict(id)?)
}

/// Resolve a single conflict, keeping the local or remote value.
/// Returns `false` if the conflict is missing or already resolved.
pub fn resolve_conflict(data_dir: &Path, id: &str, keep: ConflictKeep) -> anyhow::Result<bool> {
    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    Ok(sync_repo.resolve_conflict(id, keep)?)
}

/// Resolve a single conflict with a user-supplied merged value.
/// Returns `false` if the conflict is missing or already resolved.
pub fn resolve_conflict_with_value(
    data_dir: &Path,
    id: &str,
    value: Option<&str>,
) -> anyhow::Result<bool> {
    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    Ok(sync_repo.resolve_conflict_with_value(id, value)?)
}

/// Resolve every unresolved conflict with the same choice. Returns the count resolved.
pub fn resolve_all_conflicts(data_dir: &Path, keep: ConflictKeep) -> anyhow::Result<usize> {
    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    Ok(sync_repo.resolve_all_conflicts(keep)?)
}

/// The default device name, derived from the host name.
pub fn default_device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown-device".to_string())
}

// ── Account (1Password-style) auth flows (issue #123) ────────────────────────

/// Perform an account SRP-6a login and return the verified session result.
///
/// The SRP identity is the account `email`; the password never leaves this
/// machine. The server proof `M2` is verified before the session token is
/// trusted. A wrong password surfaces here as an authentication error with no
/// server-side disclosure of which secret was wrong.
fn account_srp_login(
    rt: &tokio::runtime::Runtime,
    client: &SyncClient,
    email: &str,
    password: &str,
    secret_key: &toku_core::SecretKey,
) -> anyhow::Result<crate::client::AccountVerifyResult> {
    use rand::RngExt;
    use sha2::Sha256;
    use srp::ClientG2048;

    let srp_client = ClientG2048::<Sha256>::new();

    let mut a = [0u8; 48];
    rand::rng().fill(&mut a);
    let a_pub_hex = hex::encode(srp_client.compute_public_ephemeral(&a));

    let challenge = rt.block_on(client.account_challenge(email, &a_pub_hex))?;

    let b_pub_bytes = hex::decode(&challenge.server_public_b)
        .context("server returned invalid hex for server_public_b")?;
    let srp_salt_bytes =
        hex::decode(&challenge.srp_salt).context("server returned invalid hex for srp_salt")?;

    // Fold the Secret Key into the SRP password input so the verifier depends on
    // both secrets (ADR-010); must match the derivation used at signup.
    let verifier_input = toku_core::srp_verifier_input(Some(secret_key.as_bytes()), password);
    let client_verifier = srp_client
        .process_reply(
            &a,
            email.as_bytes(),
            &verifier_input,
            &srp_salt_bytes,
            &b_pub_bytes,
        )
        .map_err(|_| anyhow::anyhow!("incorrect email or password"))?;

    let m1_hex = hex::encode(client_verifier.proof());
    let verify = rt.block_on(client.account_verify(&challenge.challenge_id, &m1_hex))?;

    let m2_bytes = hex::decode(&verify.server_proof_m2)
        .context("server returned invalid hex for server_proof_m2")?;
    client_verifier
        .verify_server(&m2_bytes)
        .map_err(|_| anyhow::anyhow!("server identity could not be verified (SRP M2 mismatch)"))?;

    Ok(verify)
}

/// Record an enrolled device + sync config locally and store the data key.
#[allow(clippy::too_many_arguments)]
fn finalize_device(
    data_dir: &Path,
    token_store: &TokenStore,
    server: &str,
    device_id: &str,
    library_id: &str,
    device_name: &str,
    device_session: Option<&str>,
    data_key: &SyncKey,
) -> anyhow::Result<()> {
    if let Some(token) = device_session {
        // The device-session token is the primary credential for push/pull.
        token_store
            .store(server, token)
            .context("failed to store device session token")?;
    }
    token_store
        .store_sync_key(server, data_key.as_exported_bytes())
        .context("failed to store sync key")?;

    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    let server_device_id = device_id
        .parse::<uuid::Uuid>()
        .context("server returned an invalid device_id")?;
    sync_repo.get_or_create_device_with_id(server_device_id, device_name)?;

    let mut config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    config.sync = Some(toku_core::SyncConfig {
        server: server.to_string(),
        library_id: library_id.to_string(),
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        encryption: true,
    });
    config
        .save(data_dir)
        .map_err(|e| anyhow::anyhow!("failed to save config: {e}"))?;
    Ok(())
}

/// Record that this device has completed a bootstrap.
fn mark_bootstrapped(data_dir: &Path) -> anyhow::Result<()> {
    let db = open_db(data_dir)?;
    SyncRepository::new(&db)
        .mark_bootstrapped()
        .context("failed to record bootstrap state")?;
    Ok(())
}

/// Whether this device has already completed a bootstrap.
fn is_bootstrapped(data_dir: &Path) -> anyhow::Result<bool> {
    let db = open_db(data_dir)?;
    Ok(SyncRepository::new(&db).is_bootstrapped()?)
}

/// First-opt-in op-backfill (#199, ADR-013 D2): synthesize `Create` ops for every
/// pre-existing syncable row and push them through the normal pipeline.
///
/// Layered at the opt-in boundary — after `finalize_device` has configured the
/// device identity, token, and data key, and before the caller returns — not
/// inside `push`. Idempotent: re-running never duplicates (see
/// [`toku_db::backfill_sync_ops`]). Returns a report so the caller can tell the
/// user exactly what reached the server.
fn run_backfill(data_dir: &Path) -> anyhow::Result<BackfillReport> {
    let counts = {
        let db = open_db(data_dir)?;
        toku_db::backfill_sync_ops(&db).context("failed to backfill pre-existing state")?
    };

    // Drain the freshly-staged ops (plus any already pending) to the server.
    let pushed = if counts.total() > 0 {
        push(data_dir)?.accepted
    } else {
        0
    };

    Ok(BackfillReport {
        books: counts.books,
        sessions: counts.sessions,
        progress: counts.progress,
        tags: counts.tags,
        ops_total: counts.total(),
        pushed,
    })
}

/// Reconstruct the account key hierarchy from a server key bundle and unwrap the
/// leaf data key with the password + Secret Key.
///
/// A failure here (after a successful SRP login) means the Secret Key — or the
/// password used for the key hierarchy — was wrong. The error is deliberately
/// non-specific about which secret failed.
fn unlock_data_key_from_bundle(
    bundle: &crate::client::AccountKeyBundle,
    password: &str,
    secret_key: &toku_core::SecretKey,
) -> anyhow::Result<SyncKey> {
    let kdf: toku_core::AccountKdfParams =
        serde_json::from_str(&bundle.kdf_params).context("invalid kdf_params in key bundle")?;
    let wrapped_private_key: toku_core::WrappedAccountPrivateKey =
        serde_json::from_str(&bundle.wrapped_private_key)
            .context("invalid wrapped_private_key in key bundle")?;
    let wrapped_data_key: toku_core::WrappedDataKey =
        serde_json::from_str(&bundle.wrapped_data_key)
            .context("invalid wrapped_data_key in key bundle")?;

    let account_keys = toku_core::AccountKeys {
        version: 1,
        kdf,
        public_key: bundle.account_public_key.clone(),
        wrapped_private_key,
        wrapped_data_key,
    };

    account_keys
        .unlock_data_key(password, secret_key.as_bytes())
        .map_err(|_| anyhow::anyhow!("incorrect password or Secret Key"))
}

/// Create an account on `server`, build the key hierarchy, enroll this device,
/// and persist session + key material. Returns the formatted Secret Key so the
/// caller can render the Emergency Kit exactly once.
pub fn signup(
    data_dir: &Path,
    server: &str,
    email: &str,
    password: &str,
    secret_key: &toku_core::SecretKey,
    device_name: Option<String>,
) -> anyhow::Result<SignupOutcome> {
    use rand::RngExt;
    use sha2::Sha256;
    use srp::ClientG2048;

    if password.is_empty() {
        anyhow::bail!("password cannot be empty");
    }

    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let client = SyncClient::new(server)?;
    let device_name = device_name.unwrap_or_else(default_device_name);

    // ── Build the account key hierarchy (zero-knowledge) ─────────────────────
    let (account_keys, data_key) = toku_core::AccountKeys::create(password, secret_key.as_bytes())
        .map_err(|e| anyhow::anyhow!("failed to build account keys: {e}"))?;

    let kdf_params =
        serde_json::to_string(&account_keys.kdf).context("failed to serialize kdf_params")?;
    let wrapped_private_key = serde_json::to_string(&account_keys.wrapped_private_key)
        .context("failed to serialize wrapped_private_key")?;
    let wrapped_data_key = serde_json::to_string(&account_keys.wrapped_data_key)
        .context("failed to serialize wrapped_data_key")?;

    // ── SRP verifier (identity = email, folds in the Secret Key per ADR-010) ──
    let srp_client = ClientG2048::<Sha256>::new();
    let mut srp_salt = [0u8; 16];
    rand::rng().fill(&mut srp_salt);
    let srp_salt_hex = hex::encode(srp_salt);
    let verifier_input = toku_core::srp_verifier_input(Some(secret_key.as_bytes()), password);
    let srp_verifier_hex =
        hex::encode(srp_client.compute_verifier(email.as_bytes(), &verifier_input, &srp_salt));

    let signup = rt.block_on(client.account_signup(
        email,
        &srp_salt_hex,
        &srp_verifier_hex,
        &wrapped_private_key,
        &account_keys.public_key,
        &kdf_params,
        &wrapped_data_key,
    ))?;

    // ── Log in (user session) then enroll this device ────────────────────────
    let verify = account_srp_login(&rt, &client, email, password, secret_key)?;
    token_store.store_user_session(server, &verify.session_token, &verify.expires_at)?;

    let enroll =
        rt.block_on(client.enroll_device(&verify.session_token, None, &device_name, None, None))?;

    finalize_device(
        data_dir,
        &token_store,
        server,
        &enroll.device_id,
        &enroll.library_id,
        &device_name,
        enroll.session_token.as_deref(),
        &data_key,
    )?;

    // First opt-in: backfill any pre-existing local library so it reaches the
    // server without a manual `compact` (D2). The first device always owns a
    // fresh, active library, so this always runs. A successful backfill also
    // counts as this device's bootstrap (nothing to restore from an empty
    // server), so a later routine `login` won't re-run the deferred restore.
    let backfill = run_backfill(data_dir)?;
    mark_bootstrapped(data_dir)?;

    Ok(SignupOutcome {
        user_id: signup.user_id,
        email: signup.email,
        role: signup.role,
        device_id: enroll.device_id,
        library_id: enroll.library_id,
        device_name,
        server: server.to_string(),
        device_status: enroll.status,
        secret_key: secret_key.format(),
        backfill,
    })
}

/// Log in on an already-enrolled device: refresh the user session, unwrap the
/// leaf data key (via the #143 key bundle), and refresh this device's session.
pub fn login(
    data_dir: &Path,
    server: &str,
    email: &str,
    password: &str,
    secret_key: &toku_core::SecretKey,
) -> anyhow::Result<LoginOutcome> {
    if password.is_empty() {
        anyhow::bail!("password cannot be empty");
    }

    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let client = SyncClient::new(server)?;

    let verify = account_srp_login(&rt, &client, email, password, secret_key)?;
    token_store.store_user_session(server, &verify.session_token, &verify.expires_at)?;

    // Unwrap and store the leaf data key. Requires the #143 endpoint; when it is
    // unavailable we still complete the login (session is valid) but report that
    // the data key was not unlocked so the caller can warn.
    let mut data_key_unlocked = false;
    match rt.block_on(client.account_keys(&verify.session_token)) {
        Ok(bundle) => {
            let data_key = unlock_data_key_from_bundle(&bundle, password, secret_key)?;
            token_store
                .store_sync_key(server, data_key.as_exported_bytes())
                .context("failed to store sync key")?;
            data_key_unlocked = true;
        }
        Err(e) => {
            // Surface a wrong-secret error, but tolerate a missing endpoint
            // (pre-#143 servers) so login still establishes the session.
            let msg = e.to_string();
            if msg.contains("incorrect password or Secret Key") {
                return Err(e);
            }
        }
    }

    // Refresh this device's session token if we know our device id. Track
    // whether a device session is available so we can run a deferred new-device
    // bootstrap below (D3): an approval-pending device has no session at enroll
    // time, so its bootstrap waits for the first post-approval login.
    let mut device_session_ready = false;
    if let Some(sync_config) = toku_core::TokuConfig::load(data_dir)
        .unwrap_or_default()
        .sync
        && sync_config.server == server
        && !sync_config.device_id.is_empty()
        && let Ok(session) =
            rt.block_on(client.create_device_session(&verify.session_token, &sync_config.device_id))
    {
        token_store.store(server, &session.session_token)?;
        device_session_ready = true;
    }

    // Deferred bootstrap: the first login that mints a device session for a
    // device that has never bootstrapped restores the prior library. Idempotent
    // and gated on the `bootstrapped` marker so routine logins don't re-run it.
    let mut bootstrap_result = None;
    if data_key_unlocked && device_session_ready && !is_bootstrapped(data_dir)? {
        bootstrap_result = Some(bootstrap(data_dir, false)?);
    }

    Ok(LoginOutcome {
        user_id: verify.user_id,
        email: email.to_string(),
        role: verify.role,
        server: server.to_string(),
        data_key_unlocked,
        bootstrap: bootstrap_result,
    })
}

/// Join an existing account from a new device: SRP login, unwrap the shared data
/// key, then enroll this device (handling the optional approval flow).
pub fn enroll(
    data_dir: &Path,
    server: &str,
    email: &str,
    password: &str,
    secret_key: &toku_core::SecretKey,
    device_name: Option<String>,
    library_id: Option<String>,
) -> anyhow::Result<EnrollOutcome> {
    if password.is_empty() {
        anyhow::bail!("password cannot be empty");
    }

    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let client = SyncClient::new(server)?;
    let device_name = device_name.unwrap_or_else(default_device_name);

    let verify = account_srp_login(&rt, &client, email, password, secret_key)?;
    token_store.store_user_session(server, &verify.session_token, &verify.expires_at)?;

    // Recover the shared library data key the zero-knowledge way (#143).
    let bundle = rt
        .block_on(client.account_keys(&verify.session_token))
        .context("could not fetch account key bundle (requires the #143 account-keys endpoint)")?;
    let data_key = unlock_data_key_from_bundle(&bundle, password, secret_key)?;

    let enroll = rt.block_on(client.enroll_device(
        &verify.session_token,
        library_id.as_deref(),
        &device_name,
        None,
        None,
    ))?;

    // `pending` devices have no session token until an existing device approves
    // them; we still record the device + key material so a later `login` (which
    // mints the session via create_device_session) just works.
    finalize_device(
        data_dir,
        &token_store,
        server,
        &enroll.device_id,
        &enroll.library_id,
        &device_name,
        enroll.session_token.as_deref(),
        &data_key,
    )?;

    let is_active = enroll.session_token.is_some();
    // A fresh library (caller passed no library_id) created from a device that
    // already holds local data is a first opt-in: backfill it (D2). Joining an
    // existing library instead restores from the server via bootstrap (D3).
    let fresh_library = library_id.is_none();

    let mut backfill = None;
    let mut bootstrap_result = None;
    if is_active {
        if fresh_library {
            // Nothing on the server yet — push local state up. A self-pull would
            // only re-fetch our own just-pushed ops, so we skip bootstrap and
            // just record that this device is provisioned.
            backfill = Some(run_backfill(data_dir)?);
            mark_bootstrapped(data_dir)?;
        } else {
            // Joining an existing library: restore the prior state through the
            // normal new-device bootstrap path.
            bootstrap_result = Some(bootstrap(data_dir, false)?);
        }
    }

    Ok(EnrollOutcome {
        user_id: verify.user_id,
        email: email.to_string(),
        device_id: enroll.device_id,
        library_id: enroll.library_id,
        device_name,
        server: server.to_string(),
        device_status: enroll.status,
        backfill,
        bootstrap: bootstrap_result,
    })
}

/// Re-encrypt one wire op from the legacy single-key (or plaintext) world into a
/// fresh data key, preserving all metadata. `old_key` is `None` when the relay
/// stored plaintext; an encrypted op then is an error (we can't decrypt it).
fn reencrypt_wire_op(
    wire_op: &WireOp,
    old_key: Option<&SyncKey>,
    new_key: &SyncKey,
) -> anyhow::Result<WireOp> {
    let mut out = wire_op.clone();
    if wire_op.payload.is_object() && wire_op.payload.get("ev").is_some() {
        let envelope: toku_core::EncryptedEnvelope =
            serde_json::from_value(wire_op.payload.clone())
                .context("invalid encrypted envelope")?;
        let entity_type: toku_core::EntityType = wire_op
            .entity_type
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid entity_type"))?;
        let entity_id: uuid::Uuid = wire_op
            .entity_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid entity_id: {e}"))?;
        let op_type: toku_core::OpType = wire_op
            .op_type
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid op_type"))?;
        let old = old_key.ok_or_else(|| {
            anyhow::anyhow!(
                "op {} is encrypted but no old key is available",
                wire_op.op_id
            )
        })?;
        let plaintext =
            toku_core::decrypt_fields(old, &envelope, &entity_type, &entity_id, &op_type)
                .map_err(|e| anyhow::anyhow!("decryption failed for op {}: {e}", wire_op.op_id))?;
        let new_envelope =
            toku_core::encrypt_fields(new_key, &plaintext, &entity_type, &entity_id, &op_type)
                .map_err(|e| anyhow::anyhow!("re-encryption failed: {e}"))?;
        out.payload = serde_json::to_value(&new_envelope)?;
    } else if !wire_op.payload.is_null() {
        // Legacy plaintext op: encrypt it for the first time so the migrated
        // server holds only zero-knowledge ciphertext.
        let entity_type: toku_core::EntityType = wire_op
            .entity_type
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid entity_type"))?;
        let entity_id: uuid::Uuid = wire_op
            .entity_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid entity_id: {e}"))?;
        let op_type: toku_core::OpType = wire_op
            .op_type
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid op_type"))?;
        let new_envelope = toku_core::encrypt_fields(
            new_key,
            &wire_op.payload,
            &entity_type,
            &entity_id,
            &op_type,
        )
        .map_err(|e| anyhow::anyhow!("encryption failed for op {}: {e}", wire_op.op_id))?;
        out.payload = serde_json::to_value(&new_envelope)?;
    }
    Ok(out)
}

/// One-time upgrade from the relay model to the account/key-hierarchy model
/// (issue #126). Creates the account (first account becomes admin and adopts
/// orphan libraries/devices server-side), re-binds this device to a fresh
/// account session, generates a new library data key, and rekeys every server
/// op (and any snapshot) from the legacy single passphrase — or plaintext — into
/// zero-knowledge ciphertext under the new key. Idempotent-friendly: safe to
/// re-run for a partially-migrated instance (a duplicate email signup is
/// rejected; existing data key login still rekeys).
pub fn migrate(
    data_dir: &Path,
    email: &str,
    password: &str,
    secret_key: &toku_core::SecretKey,
) -> anyhow::Result<MigrateOutcome> {
    if password.is_empty() {
        anyhow::bail!("password cannot be empty");
    }

    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    let sync_config = require_sync(&config)?;
    let server = sync_config.server.clone();
    let library_id = sync_config.library_id.clone();
    let device_id = sync_config.device_id.clone();
    if device_id.is_empty() {
        anyhow::bail!("no device id in sync config; cannot migrate this install");
    }
    let client = SyncClient::new(&server)?;

    // Old encryption key (passphrase-derived) if this relay used encryption.
    let old_key = load_encryption_key(&token_store, &server, sync_config)?;

    // Build the fresh account key hierarchy + data key.
    let (account_keys, data_key) = toku_core::AccountKeys::create(password, secret_key.as_bytes())
        .map_err(|e| anyhow::anyhow!("failed to build account keys: {e}"))?;
    let kdf_params =
        serde_json::to_string(&account_keys.kdf).context("failed to serialize kdf_params")?;
    let wrapped_private_key = serde_json::to_string(&account_keys.wrapped_private_key)
        .context("failed to serialize wrapped_private_key")?;
    let wrapped_data_key = serde_json::to_string(&account_keys.wrapped_data_key)
        .context("failed to serialize wrapped_data_key")?;

    // SRP verifier (identity = email, folds in the Secret Key per ADR-010).
    let (srp_salt_hex, srp_verifier_hex) = {
        use rand::RngExt;
        use sha2::Sha256;
        use srp::ClientG2048;
        let srp_client = ClientG2048::<Sha256>::new();
        let mut salt = [0u8; 16];
        rand::rng().fill(&mut salt);
        let verifier_input = toku_core::srp_verifier_input(Some(secret_key.as_bytes()), password);
        (
            hex::encode(salt),
            hex::encode(srp_client.compute_verifier(email.as_bytes(), &verifier_input, &salt)),
        )
    };

    let signup = rt.block_on(client.account_signup(
        email,
        &srp_salt_hex,
        &srp_verifier_hex,
        &wrapped_private_key,
        &account_keys.public_key,
        &kdf_params,
        &wrapped_data_key,
    ))?;

    // Account session, then a device session for this already-registered device
    // (now adopted under the admin account).
    let verify = account_srp_login(&rt, &client, email, password, secret_key)?;
    token_store.store_user_session(&server, &verify.session_token, &verify.expires_at)?;
    let device_session =
        rt.block_on(client.create_device_session(&verify.session_token, &device_id))?;
    token_store.store(&server, &device_session.session_token)?;
    let token = device_session.session_token;

    // Rekey all server ops + snapshot under the fresh data key.
    let (ops_reencrypted, ops_replaced) = rt.block_on(async {
        let pull = client.pull_all_ops(&token).await?;
        let new_salt = SyncKey::generate_salt()?;
        let new_salt_b64 = base64::engine::general_purpose::STANDARD.encode(new_salt);
        let mut reencrypted = Vec::with_capacity(pull.ops.len());
        for wire_op in &pull.ops {
            reencrypted.push(reencrypt_wire_op(wire_op, old_key.as_ref(), &data_key)?);
        }
        let count = reencrypted.len();
        let rekey = client.rekey(&token, &new_salt_b64, &reencrypted).await?;
        if let Some(old) = old_key.as_ref()
            && let Some(snap) = client.download_snapshot(&token).await?
        {
            let old_env: toku_core::EncryptedEnvelope =
                serde_json::from_str(&snap.snapshot_json)
                    .context("stored snapshot is not an encrypted envelope")?;
            let plain = toku_core::decrypt_snapshot(old, &old_env)
                .map_err(|e| anyhow::anyhow!("failed to decrypt snapshot: {e}"))?;
            let new_env = toku_core::encrypt_snapshot(&data_key, &plain)
                .map_err(|e| anyhow::anyhow!("failed to re-encrypt snapshot: {e}"))?;
            let blob = serde_json::to_string(&new_env)?;
            client
                .upload_snapshot(&token, &blob, &snap.hlc_at_snapshot)
                .await?;
        }
        anyhow::Ok((count, rekey.ops_replaced))
    })?;

    // Persist the new data key and mark encryption enabled.
    token_store.store_sync_key(&server, data_key.as_exported_bytes())?;
    let mut config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    config.sync = Some(toku_core::SyncConfig {
        server: server.clone(),
        library_id: library_id.clone(),
        device_id: device_id.clone(),
        device_name: sync_config.device_name.clone(),
        encryption: true,
    });
    config
        .save(data_dir)
        .map_err(|e| anyhow::anyhow!("failed to save config: {e}"))?;

    Ok(MigrateOutcome {
        user_id: signup.user_id,
        email: signup.email,
        role: signup.role,
        server,
        library_id,
        device_id,
        adopted_libraries: signup.adopted_libraries,
        adopted_devices: signup.adopted_devices,
        ops_reencrypted,
        ops_replaced,
        had_encryption: old_key.is_some(),
        secret_key: secret_key.format(),
    })
}

/// List the authenticated user's devices (user-scoped). Requires a stored user
/// session (from `signup`/`login`/`enroll`).
pub fn account_devices(
    data_dir: &Path,
    server: &str,
) -> anyhow::Result<Vec<crate::client::AccountDeviceInfo>> {
    let rt = build_runtime()?;
    let token_store = TokenStore::new(data_dir);
    let client = SyncClient::new(server)?;
    let session = token_store.load_user_session(server)?.ok_or_else(|| {
        anyhow::anyhow!("not logged in to {server}. Run `toku sync login` first.")
    })?;
    rt.block_on(client.list_account_devices(&session))
}
