//! Wire format conversion between `SyncOp` (domain) and `WireOp` (HTTP).
//!
//! The op's metadata (ids, hlc, entity/op type) always travels in cleartext.
//! The `payload` carries either the cleartext `fields` object or — when
//! client-side encryption is enabled — the [`EncryptedEnvelope`] (per ADR-008).
//! An encrypted payload is distinguished by its `ev` (envelope version) key,
//! which never collides with a domain field name. This matches the convention
//! already used by the `rekey` flow.

use toku_core::EncryptedEnvelope;
use toku_core::sync::{EntityType, HlcTimestamp, OpType, SyncOp};
use uuid::Uuid;

use crate::client::WireOp;

/// Convert a domain `SyncOp` to wire format for pushing to the server.
pub fn to_wire(op: &SyncOp) -> WireOp {
    let payload = if let Some(ref envelope) = op.encrypted {
        serde_json::to_value(envelope).unwrap_or(serde_json::Value::Null)
    } else {
        op.fields.clone().unwrap_or(serde_json::Value::Null)
    };

    WireOp {
        op_id: op.op_id.to_string(),
        device_id: op.device_id.to_string(),
        hlc: op.hlc.to_canonical(),
        entity_type: op.entity_type.as_str().to_string(),
        entity_id: op.entity_id.to_string(),
        op_type: op.op_type.as_str().to_string(),
        payload,
    }
}

/// Returns `true` if a wire payload carries an encrypted envelope rather than
/// cleartext fields.
fn is_encrypted_payload(payload: &serde_json::Value) -> bool {
    payload.is_object() && payload.get("ev").is_some()
}

/// Convert a wire format `WireOp` to domain `SyncOp` after pulling from the server.
pub fn from_wire(wire: &WireOp) -> anyhow::Result<SyncOp> {
    let op_id: Uuid = wire
        .op_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid op_id: {e}"))?;
    let device_id: Uuid = wire
        .device_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid device_id: {e}"))?;
    let hlc: HlcTimestamp = wire
        .hlc
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid hlc: {}", wire.hlc))?;
    let entity_type: EntityType = wire
        .entity_type
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid entity_type: {}", wire.entity_type))?;
    let entity_id: Uuid = wire
        .entity_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid entity_id: {e}"))?;
    let op_type: OpType = wire
        .op_type
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid op_type: {}", wire.op_type))?;

    let (fields, encrypted) = if is_encrypted_payload(&wire.payload) {
        let envelope: EncryptedEnvelope = serde_json::from_value(wire.payload.clone())
            .map_err(|e| anyhow::anyhow!("invalid encrypted envelope: {e}"))?;
        (None, Some(envelope))
    } else if wire.payload.is_null() {
        (None, None)
    } else {
        (Some(wire.payload.clone()), None)
    };

    // Build a SyncOp with checksum recomputed from the parsed contents.
    let mut op = SyncOp {
        v: 1,
        op_id,
        device_id,
        hlc,
        entity_type,
        entity_id,
        op_type,
        fields,
        encrypted,
        checksum: String::new(),
        created_at: chrono::Utc::now(),
    };
    op.checksum = op.compute_checksum();
    Ok(op)
}
