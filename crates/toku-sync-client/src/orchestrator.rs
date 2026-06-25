//! High-level sync orchestration shared by the CLI and FFI frontends.
//!
//! These functions encapsulate the full push/pull/init/status flows that previously
//! lived inline in the `toku-cli` binary. They take a data directory, perform the
//! network + database work, and return structured outcomes. They never print and never
//! prompt — callers (CLI, FFI) are responsible for I/O and presentation.

use std::path::Path;

use anyhow::Context;
use serde::Serialize;
use toku_db::{ConflictKeep, Database, SyncConflict, SyncRepository};

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
    pub pulled: usize,
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

fn build_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")
}

fn open_db(data_dir: &Path) -> anyhow::Result<Database> {
    Database::open(&data_dir.join("toku.db")).context("failed to open database")
}

fn require_sync(config: &toku_core::TokuConfig) -> anyhow::Result<&toku_core::SyncConfig> {
    config
        .sync
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("sync is not configured. Run sync init first."))
}

fn require_token(token_store: &TokenStore, server: &str) -> anyhow::Result<String> {
    token_store
        .load(server)?
        .ok_or_else(|| anyhow::anyhow!("no auth token found for {server}. Run sync init first."))
}

/// Initialize sync: register the device with the server, persist the auth token and
/// sync config, optionally enabling client-side encryption with the given passphrase.
///
/// Unlike the CLI command, this function does not prompt: when `passphrase` is `Some`,
/// encryption is enabled using that passphrase; when `None`, encryption is disabled.
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
    let resp = rt.block_on(client.register(&library_id, &device_name))?;

    token_store
        .store(server, &resp.auth_token)
        .context("failed to store auth token")?;

    let encryption_enabled = match passphrase {
        Some(pass) if !pass.is_empty() => {
            let salt = toku_core::SyncKey::generate_salt();
            let key = toku_core::SyncKey::derive(pass, &salt)
                .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
            token_store
                .store_sync_key(server, key.as_exported_bytes())
                .context("failed to store sync key")?;
            true
        }
        _ => false,
    };

    let db = open_db(data_dir)?;
    let sync_repo = SyncRepository::new(&db);
    sync_repo.get_or_create_device(&device_name)?;

    let mut config = toku_core::TokuConfig::load(data_dir).unwrap_or_default();
    config.sync = Some(toku_core::SyncConfig {
        server: server.to_string(),
        library_id: resp.library_id.clone(),
        device_id: resp.device_id.clone(),
        device_name: device_name.clone(),
        encryption: encryption_enabled,
    });
    config
        .save(data_dir)
        .map_err(|e| anyhow::anyhow!("failed to save config: {e}"))?;

    Ok(InitOutcome {
        device_id: resp.device_id,
        library_id: resp.library_id,
        device_name,
        server: server.to_string(),
        encryption: encryption_enabled,
    })
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
    let wire_ops: Vec<WireOp> = unpushed.iter().map(wire::to_wire).collect();

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
    let client = SyncClient::new(server)?;

    let mut cursor = sync_repo.get_cursor("pull_cursor")?;
    let mut total_pulled = 0usize;

    loop {
        let result = rt.block_on(client.pull_ops(&token, cursor.as_deref()))?;
        if result.ops.is_empty() {
            break;
        }
        for wire_op in &result.ops {
            let sync_op = wire::from_wire(wire_op).context("failed to parse remote op")?;
            sync_repo.insert_remote_op(&sync_op)?;
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
        cursor,
    })
}

/// Report the current sync status: configuration, pending ops, cursors, and the
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
