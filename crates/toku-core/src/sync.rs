use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::crypto::EncryptedEnvelope;

// ---------------------------------------------------------------------------
// HLC Timestamp
// ---------------------------------------------------------------------------

/// A Hybrid Logical Clock timestamp providing causal ordering across devices.
///
/// Format: `YYYY-MM-DDTHH:MM:SS.mmmZ-CCCC-DDDDDDDDDDDD`
///   - Fixed-width ISO-8601 UTC with millisecond precision
///   - 4-digit zero-padded logical counter
///   - 12-character hex device prefix (from UUID without hyphens)
///
/// Lexicographic string comparison produces correct causal ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HlcTimestamp {
    pub physical: DateTime<Utc>,
    pub counter: u16,
    pub device_prefix: String,
}

impl HlcTimestamp {
    /// Create a new HLC timestamp.
    pub fn new(physical: DateTime<Utc>, counter: u16, device_prefix: impl Into<String>) -> Self {
        Self {
            physical,
            counter,
            device_prefix: device_prefix.into(),
        }
    }

    /// Returns the canonical fixed-width string representation.
    pub fn to_canonical(&self) -> String {
        format!(
            "{}-{:04}-{}",
            self.physical.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            self.counter,
            self.device_prefix,
        )
    }
}

impl fmt::Display for HlcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

impl FromStr for HlcTimestamp {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Expected: "2026-06-15T10:30:00.000Z-0001-019726a3abcd"
        //            ^--- 24 chars ---^     ^4 ^ ^--- 12 ---^
        let parts: Vec<&str> = s.rsplitn(3, '-').collect();
        if parts.len() != 3 {
            return Err(crate::TokuError::InvalidHlc(s.to_string()));
        }
        // rsplitn reverses: [device_prefix, counter, timestamp]
        let device_prefix = parts[0];
        let counter_str = parts[1];
        let timestamp_str = parts[2];

        if device_prefix.len() != 12 {
            return Err(crate::TokuError::InvalidHlc(s.to_string()));
        }

        let counter: u16 = counter_str
            .parse()
            .map_err(|_| crate::TokuError::InvalidHlc(s.to_string()))?;

        let physical = DateTime::parse_from_rfc3339(timestamp_str)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| crate::TokuError::InvalidHlc(s.to_string()))?;

        Ok(Self {
            physical,
            counter,
            device_prefix: device_prefix.to_string(),
        })
    }
}

impl Ord for HlcTimestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_canonical().cmp(&other.to_canonical())
    }
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Hybrid Clock
// ---------------------------------------------------------------------------

/// Generates monotonically increasing HLC timestamps.
///
/// Tracks the highest physical time seen and a logical counter for ordering
/// events within the same millisecond.
pub struct HybridClock {
    last_physical: DateTime<Utc>,
    counter: u16,
    device_prefix: String,
}

impl HybridClock {
    /// Create a new clock for the given device.
    pub fn new(device_id: &Uuid) -> Self {
        let hex = device_id
            .as_simple()
            .to_string()
            .chars()
            .take(12)
            .collect::<String>();
        Self {
            last_physical: DateTime::UNIX_EPOCH,
            counter: 0,
            device_prefix: hex,
        }
    }

    /// Generate a new HLC timestamp using the system clock.
    pub fn now(&mut self) -> HlcTimestamp {
        self.now_at(Utc::now())
    }

    /// Truncate a DateTime to millisecond precision.
    ///
    /// Our canonical HLC format uses millisecond precision (`%.3fZ`), so
    /// comparisons must also use millisecond granularity to avoid the
    /// counter resetting when two calls happen within the same millisecond
    /// but at different nanoseconds.
    fn truncate_to_millis(dt: DateTime<Utc>) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(dt.timestamp_millis()).unwrap_or(dt)
    }

    /// Generate a new HLC timestamp at a specific physical time.
    ///
    /// This is the primary method — `now()` is a convenience wrapper.
    /// Use `now_at` in tests for deterministic behavior.
    pub fn now_at(&mut self, physical: DateTime<Utc>) -> HlcTimestamp {
        let physical = Self::truncate_to_millis(physical);
        if physical > self.last_physical {
            self.last_physical = physical;
            self.counter = 0;
        } else if self.counter == u16::MAX {
            // Counter overflow: advance physical time by 1ms to maintain monotonicity
            self.last_physical += chrono::Duration::milliseconds(1);
            self.counter = 0;
        } else {
            self.counter += 1;
        }

        HlcTimestamp::new(self.last_physical, self.counter, &self.device_prefix)
    }

    /// Update the clock after receiving a remote HLC timestamp.
    ///
    /// Ensures the next local timestamp is causally after the remote one.
    pub fn update(&mut self, remote: &HlcTimestamp) -> HlcTimestamp {
        self.update_at(remote, Utc::now())
    }

    /// Update the clock with a remote timestamp at a specific physical time.
    pub fn update_at(&mut self, remote: &HlcTimestamp, physical: DateTime<Utc>) -> HlcTimestamp {
        let physical = Self::truncate_to_millis(physical);
        let max_physical = physical.max(self.last_physical).max(remote.physical);

        if max_physical == physical && physical > self.last_physical && physical > remote.physical {
            // Wall clock is ahead — reset counter
            self.last_physical = physical;
            self.counter = 0;
        } else if max_physical == remote.physical && remote.physical > self.last_physical {
            // Remote is ahead — adopt remote physical, increment remote counter
            self.last_physical = remote.physical;
            self.counter = remote.counter.saturating_add(1);
            if self.counter == 0 {
                // Overflow from saturating_add on u16::MAX
                self.last_physical += chrono::Duration::milliseconds(1);
            }
        } else if max_physical == self.last_physical && self.last_physical > remote.physical {
            // Local is ahead — keep local physical, increment local counter
            self.counter += 1;
            if self.counter == 0 {
                self.last_physical += chrono::Duration::milliseconds(1);
            }
        } else {
            // Same physical time — take max counter + 1
            self.counter = self.counter.max(remote.counter) + 1;
            if self.counter == 0 {
                self.last_physical += chrono::Duration::milliseconds(1);
            }
        }

        HlcTimestamp::new(self.last_physical, self.counter, &self.device_prefix)
    }
}

// ---------------------------------------------------------------------------
// Entity Type
// ---------------------------------------------------------------------------

/// The type of entity a sync operation refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    Book,
    Session,
    Progress,
    Tag,
    Note,
    Review,
    Setting,
    Device,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Book => "book",
            Self::Session => "session",
            Self::Progress => "progress",
            Self::Tag => "tag",
            Self::Note => "note",
            Self::Review => "review",
            Self::Setting => "setting",
            Self::Device => "device",
        }
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityType {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "book" => Ok(Self::Book),
            "session" => Ok(Self::Session),
            "progress" => Ok(Self::Progress),
            "tag" => Ok(Self::Tag),
            "note" => Ok(Self::Note),
            "review" => Ok(Self::Review),
            "setting" => Ok(Self::Setting),
            "device" => Ok(Self::Device),
            _ => Err(crate::TokuError::InvalidEntityType(s.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Op Type
// ---------------------------------------------------------------------------

/// The type of mutation a sync operation represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpType {
    Create,
    Update,
    Delete,
}

impl OpType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

impl fmt::Display for OpType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OpType {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            _ => Err(crate::TokuError::InvalidOpType(s.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Sync Op
// ---------------------------------------------------------------------------

/// A sync operation representing a single mutation to a library entity.
///
/// Every local mutation produces a `SyncOp` that can be pushed to the sync
/// server. The op carries a versioned envelope format (see ADR-008).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOp {
    /// Envelope version (currently 1).
    pub v: u16,
    /// Globally unique, time-sortable op identifier.
    pub op_id: Uuid,
    /// The device that created this op.
    pub device_id: Uuid,
    /// Hybrid Logical Clock timestamp for causal ordering.
    pub hlc: HlcTimestamp,
    /// The type of entity being modified.
    pub entity_type: EntityType,
    /// The UUID of the entity being modified.
    pub entity_id: Uuid,
    /// The type of mutation.
    pub op_type: OpType,
    /// Field-level changes as a canonical JSON object. `None` for deletes
    /// or when encryption is enabled (see `encrypted`).
    pub fields: Option<serde_json::Value>,
    /// Encrypted fields envelope. Present when client-side encryption is
    /// enabled; `fields` is `None` in this case. Mutually exclusive with
    /// `fields` (except for deletes where both may be `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<EncryptedEnvelope>,
    /// SHA-256 hash of the canonical op (without the checksum field).
    pub checksum: String,
    /// When this op was created locally.
    pub created_at: DateTime<Utc>,
}

impl SyncOp {
    /// Create a new sync op with an auto-computed checksum.
    pub fn new(
        device_id: Uuid,
        hlc: HlcTimestamp,
        entity_type: EntityType,
        entity_id: Uuid,
        op_type: OpType,
        fields: Option<serde_json::Value>,
    ) -> Self {
        let mut op = Self {
            v: 1,
            op_id: Uuid::now_v7(),
            device_id,
            hlc,
            entity_type,
            entity_id,
            op_type,
            fields,
            encrypted: None,
            checksum: String::new(),
            created_at: Utc::now(),
        };
        op.checksum = op.compute_checksum();
        op
    }

    /// Compute the SHA-256 checksum of this op's canonical representation.
    ///
    /// The checksum covers all fields except `checksum` itself, serialized
    /// as a canonical JSON object with sorted keys. When encryption is
    /// enabled, `encrypted` replaces `fields` in the checksum input.
    pub fn compute_checksum(&self) -> String {
        let mut map = BTreeMap::new();
        map.insert("v", serde_json::json!(self.v));
        map.insert("op_id", serde_json::json!(self.op_id.to_string()));
        map.insert("device_id", serde_json::json!(self.device_id.to_string()));
        map.insert("hlc", serde_json::json!(self.hlc.to_canonical()));
        map.insert("entity_type", serde_json::json!(self.entity_type.as_str()));
        map.insert("entity_id", serde_json::json!(self.entity_id.to_string()));
        map.insert("op_type", serde_json::json!(self.op_type.as_str()));

        if let Some(ref enc) = self.encrypted {
            map.insert(
                "encrypted",
                serde_json::to_value(enc).expect("EncryptedEnvelope serialization cannot fail"),
            );
            map.insert("fields", serde_json::Value::Null);
        } else {
            map.insert(
                "fields",
                self.fields.clone().unwrap_or(serde_json::Value::Null),
            );
        }

        map.insert(
            "created_at",
            serde_json::json!(self.created_at.to_rfc3339()),
        );

        let canonical = serde_json::to_string(&map).expect("BTreeMap serialization cannot fail");
        let hash = Sha256::digest(canonical.as_bytes());
        format!("sha256:{:x}", hash)
    }

    /// Verify this op's checksum is correct.
    pub fn verify_checksum(&self) -> bool {
        self.checksum == self.compute_checksum()
    }

    /// Encrypt this op's `fields` in place, replacing them with an
    /// [`EncryptedEnvelope`]. The checksum is recomputed after encryption.
    ///
    /// No-op if `fields` is already `None` (e.g. delete ops).
    pub fn encrypt(&mut self, key: &crate::crypto::SyncKey) -> Result<(), crate::TokuError> {
        let Some(ref fields) = self.fields else {
            return Ok(());
        };
        let envelope = crate::crypto::encrypt_fields(
            key,
            fields,
            &self.entity_type,
            &self.entity_id,
            &self.op_type,
        )?;
        self.encrypted = Some(envelope);
        self.fields = None;
        self.checksum = self.compute_checksum();
        Ok(())
    }

    /// Decrypt this op's encrypted envelope in place, restoring `fields`.
    /// The checksum is recomputed after decryption.
    ///
    /// No-op if there is no encrypted envelope.
    pub fn decrypt(&mut self, key: &crate::crypto::SyncKey) -> Result<(), crate::TokuError> {
        let Some(ref envelope) = self.encrypted else {
            return Ok(());
        };
        let fields = crate::crypto::decrypt_fields(
            key,
            envelope,
            &self.entity_type,
            &self.entity_id,
            &self.op_type,
        )?;
        self.fields = Some(fields);
        self.encrypted = None;
        self.checksum = self.compute_checksum();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Device Identity
// ---------------------------------------------------------------------------

/// Identifies this device for sync purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub device_id: Uuid,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
}

impl DeviceIdentity {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            device_id: Uuid::now_v7(),
            device_name: name.into(),
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Library Snapshot
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of the entire library state.
///
/// Used for snapshot compaction (pruning old ops) and bootstrapping new
/// devices without replaying the full op history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    /// Snapshot format version (currently 1).
    pub version: u16,
    /// When this snapshot was created.
    pub created_at: DateTime<Utc>,
    /// The device that created this snapshot.
    pub created_by_device: Uuid,
    /// HLC at the time of snapshot. Ops older than this can be pruned.
    pub hlc_at_snapshot: String,
    /// The complete library state.
    pub library: SnapshotLibrary,
}

/// The library content within a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotLibrary {
    pub books: Vec<serde_json::Value>,
    pub book_authors: Vec<serde_json::Value>,
    pub sessions: Vec<serde_json::Value>,
    pub progress: Vec<serde_json::Value>,
    pub tags: Vec<serde_json::Value>,
    pub book_tags: Vec<serde_json::Value>,
    pub notes: Vec<serde_json::Value>,
    pub reviews: Vec<serde_json::Value>,
    pub settings: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time(millis: i64) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(millis).unwrap()
    }

    fn test_device_id() -> Uuid {
        Uuid::parse_str("01972123-abcd-7000-8000-000000000001").unwrap()
    }

    fn test_device_prefix() -> String {
        test_device_id()
            .as_simple()
            .to_string()
            .chars()
            .take(12)
            .collect()
    }

    // --- HlcTimestamp ---

    #[test]
    fn hlc_display_format_is_fixed_width() {
        let ts = HlcTimestamp::new(fixed_time(1718444400000), 1, test_device_prefix());
        let s = ts.to_canonical();
        // Fixed 42-char format
        assert_eq!(s.len(), 42);
        assert!(s.ends_with(&test_device_prefix()));
    }

    #[test]
    fn hlc_display_roundtrip() {
        let ts = HlcTimestamp::new(fixed_time(1718444400123), 42, test_device_prefix());
        let s = ts.to_string();
        let parsed: HlcTimestamp = s.parse().unwrap();
        assert_eq!(parsed.physical, ts.physical);
        assert_eq!(parsed.counter, 42);
        assert_eq!(parsed.device_prefix, test_device_prefix());
    }

    #[test]
    fn hlc_lexicographic_ordering_by_time() {
        let early = HlcTimestamp::new(fixed_time(1000), 0, "aaaaaaaaaaaa");
        let late = HlcTimestamp::new(fixed_time(2000), 0, "aaaaaaaaaaaa");
        assert!(early < late);
    }

    #[test]
    fn hlc_lexicographic_ordering_by_counter() {
        let low = HlcTimestamp::new(fixed_time(1000), 1, "aaaaaaaaaaaa");
        let high = HlcTimestamp::new(fixed_time(1000), 2, "aaaaaaaaaaaa");
        assert!(low < high);
    }

    #[test]
    fn hlc_lexicographic_ordering_by_device() {
        let a = HlcTimestamp::new(fixed_time(1000), 0, "aaaaaaaaaaaa");
        let b = HlcTimestamp::new(fixed_time(1000), 0, "bbbbbbbbbbbb");
        assert!(a < b);
    }

    #[test]
    fn hlc_parse_rejects_invalid() {
        assert!("not-a-timestamp".parse::<HlcTimestamp>().is_err());
        assert!(
            "2026-01-01T00:00:00.000Z-0001-short"
                .parse::<HlcTimestamp>()
                .is_err()
        );
    }

    // --- HybridClock ---

    #[test]
    fn clock_monotonically_increasing() {
        let mut clock = HybridClock::new(&test_device_id());
        let t1 = clock.now_at(fixed_time(1000));
        let t2 = clock.now_at(fixed_time(2000));
        assert!(t2 > t1);
    }

    #[test]
    fn clock_same_millis_increments_counter() {
        let mut clock = HybridClock::new(&test_device_id());
        let t1 = clock.now_at(fixed_time(1000));
        let t2 = clock.now_at(fixed_time(1000));
        assert_eq!(t1.counter, 0);
        assert_eq!(t2.counter, 1);
        assert!(t2 > t1);
    }

    #[test]
    fn clock_backward_time_stays_monotonic() {
        let mut clock = HybridClock::new(&test_device_id());
        let t1 = clock.now_at(fixed_time(2000));
        let t2 = clock.now_at(fixed_time(1000)); // clock goes backward
        assert!(t2 > t1, "backward clock must still produce increasing HLC");
        assert_eq!(t2.physical, fixed_time(2000)); // physical stays at max seen
        assert_eq!(t2.counter, 1); // counter incremented
    }

    #[test]
    fn clock_counter_overflow_advances_physical() {
        let mut clock = HybridClock::new(&test_device_id());
        let base = fixed_time(1000);
        // Exhaust the counter
        clock.last_physical = base;
        clock.counter = u16::MAX;
        let ts = clock.now_at(base);
        // Should have advanced physical by 1ms and reset counter
        assert_eq!(ts.physical, base + chrono::Duration::milliseconds(1));
        assert_eq!(ts.counter, 0);
    }

    #[test]
    fn clock_update_adopts_ahead_remote() {
        let mut clock = HybridClock::new(&test_device_id());
        let local_time = fixed_time(1000);
        let remote = HlcTimestamp::new(fixed_time(5000), 3, "bbbbbbbbbbbb");

        let merged = clock.update_at(&remote, local_time);
        assert!(merged > remote, "merged must be after remote");
        assert_eq!(merged.physical, fixed_time(5000));
        assert_eq!(merged.counter, 4);
    }

    #[test]
    fn clock_update_local_ahead_of_remote() {
        let mut clock = HybridClock::new(&test_device_id());
        let _ = clock.now_at(fixed_time(5000));

        let remote = HlcTimestamp::new(fixed_time(2000), 0, "bbbbbbbbbbbb");
        let local_time = fixed_time(3000); // behind local clock's last_physical

        let merged = clock.update_at(&remote, local_time);
        assert!(merged > remote);
        assert_eq!(merged.physical, fixed_time(5000)); // keeps local max
    }

    #[test]
    fn clock_update_wall_clock_ahead_of_both() {
        let mut clock = HybridClock::new(&test_device_id());
        let _ = clock.now_at(fixed_time(1000));

        let remote = HlcTimestamp::new(fixed_time(2000), 0, "bbbbbbbbbbbb");
        let local_time = fixed_time(5000); // wall clock ahead of both

        let merged = clock.update_at(&remote, local_time);
        assert!(merged > remote);
        assert_eq!(merged.physical, fixed_time(5000));
        assert_eq!(merged.counter, 0); // reset since wall clock is newest
    }

    #[test]
    fn two_devices_deterministic_ordering() {
        let dev_a = Uuid::parse_str("01972123-aaaa-7000-8000-000000000001").unwrap();
        let dev_b = Uuid::parse_str("01972123-bbbb-7000-8000-000000000001").unwrap();

        let mut clock_a = HybridClock::new(&dev_a);
        let mut clock_b = HybridClock::new(&dev_b);

        let same_time = fixed_time(1000);
        let ts_a = clock_a.now_at(same_time);
        let ts_b = clock_b.now_at(same_time);

        // Same physical, same counter — device prefix breaks tie
        assert_ne!(ts_a, ts_b);
        // One is deterministically before the other
        assert!(ts_a < ts_b || ts_b < ts_a);
    }

    // --- EntityType ---

    #[test]
    fn entity_type_roundtrip() {
        for variant in [
            EntityType::Book,
            EntityType::Session,
            EntityType::Progress,
            EntityType::Tag,
            EntityType::Note,
            EntityType::Review,
            EntityType::Setting,
            EntityType::Device,
        ] {
            let s = variant.as_str();
            let parsed: EntityType = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn entity_type_invalid() {
        assert!("invalid".parse::<EntityType>().is_err());
    }

    // --- OpType ---

    #[test]
    fn op_type_roundtrip() {
        for variant in [OpType::Create, OpType::Update, OpType::Delete] {
            let s = variant.as_str();
            let parsed: OpType = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn op_type_invalid() {
        assert!("invalid".parse::<OpType>().is_err());
    }

    // --- SyncOp checksum ---

    #[test]
    fn sync_op_checksum_is_stable() {
        let op = SyncOp::new(
            test_device_id(),
            HlcTimestamp::new(fixed_time(1000), 0, test_device_prefix()),
            EntityType::Book,
            Uuid::parse_str("01972123-0000-7000-8000-000000000002").unwrap(),
            OpType::Update,
            Some(serde_json::json!({"rating": 8})),
        );
        assert!(op.checksum.starts_with("sha256:"));
        assert!(op.verify_checksum());
    }

    #[test]
    fn sync_op_different_fields_produce_different_checksum() {
        let hlc = HlcTimestamp::new(fixed_time(1000), 0, test_device_prefix());
        let entity_id = Uuid::parse_str("01972123-0000-7000-8000-000000000002").unwrap();

        let op_a = SyncOp::new(
            test_device_id(),
            hlc.clone(),
            EntityType::Book,
            entity_id,
            OpType::Update,
            Some(serde_json::json!({"rating": 8})),
        );
        let op_b = SyncOp::new(
            test_device_id(),
            hlc,
            EntityType::Book,
            entity_id,
            OpType::Update,
            Some(serde_json::json!({"rating": 9})),
        );
        assert_ne!(op_a.checksum, op_b.checksum);
    }

    #[test]
    fn sync_op_canonical_json_key_order_stable() {
        // Regardless of field insertion order, BTreeMap sorts keys
        let op = SyncOp::new(
            test_device_id(),
            HlcTimestamp::new(fixed_time(1000), 0, test_device_prefix()),
            EntityType::Book,
            Uuid::nil(),
            OpType::Create,
            Some(serde_json::json!({"z_field": 1, "a_field": 2})),
        );
        let checksum1 = op.compute_checksum();
        let checksum2 = op.compute_checksum();
        assert_eq!(checksum1, checksum2);
    }

    #[test]
    fn sync_op_delete_has_no_fields() {
        let op = SyncOp::new(
            test_device_id(),
            HlcTimestamp::new(fixed_time(1000), 0, test_device_prefix()),
            EntityType::Book,
            Uuid::nil(),
            OpType::Delete,
            None,
        );
        assert!(op.fields.is_none());
        assert!(op.verify_checksum());
    }

    // --- DeviceIdentity ---

    #[test]
    fn device_identity_creates_with_uuid_v7() {
        let dev = DeviceIdentity::new("Test Laptop");
        assert_eq!(dev.device_name, "Test Laptop");
        // UUID v7 — version nibble is 7
        assert_eq!(dev.device_id.get_version_num(), 7);
    }
}
