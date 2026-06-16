//! Wire format conversion between `SyncOp` (domain) and `WireOp` (HTTP).

use toku_core::sync::{EntityType, HlcTimestamp, OpType, SyncOp};
use uuid::Uuid;

use super::client::WireOp;

/// Convert a domain `SyncOp` to wire format for pushing to the server.
pub fn to_wire(op: &SyncOp) -> WireOp {
    WireOp {
        op_id: op.op_id.to_string(),
        device_id: op.device_id.to_string(),
        hlc: op.hlc.to_canonical(),
        entity_type: op.entity_type.as_str().to_string(),
        entity_id: op.entity_id.to_string(),
        op_type: op.op_type.as_str().to_string(),
        payload: op.fields.clone().unwrap_or(serde_json::Value::Null),
    }
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

    let fields = if wire.payload.is_null() {
        None
    } else {
        Some(wire.payload.clone())
    };

    // Build a SyncOp with checksum recomputed from the parsed fields.
    let mut op = SyncOp {
        v: 1,
        op_id,
        device_id,
        hlc,
        entity_type,
        entity_id,
        op_type,
        fields,
        encrypted: None,
        checksum: String::new(),
        created_at: chrono::Utc::now(),
    };
    op.checksum = op.compute_checksum();
    Ok(op)
}
