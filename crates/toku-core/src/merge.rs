//! Merge outcome types for sync conflict resolution.
//!
//! These types live in `toku-core` (no I/O) so they can be shared across
//! the database merge engine and future UI conflict resolution views.

use uuid::Uuid;

use crate::sync::EntityType;

/// The result of applying a single remote sync op to the local database.
#[derive(Debug)]
pub enum MergeOutcome {
    /// The op was successfully applied to local state.
    Applied,
    /// The op was applied, but conflicts were detected and stored for user review.
    AppliedWithConflicts(Vec<MergeConflict>),
    /// The op was skipped (duplicate, stale, or not applicable).
    Skipped { reason: &'static str },
    /// The op was rejected (e.g. invalid reading status transition).
    Rejected { reason: String },
}

/// A merge conflict between local and remote state on a single field.
#[derive(Debug, Clone)]
pub struct MergeConflict {
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub field_name: String,
    pub local_value: Option<String>,
    pub remote_value: Option<String>,
    pub local_hlc: String,
    pub remote_hlc: String,
}

impl MergeOutcome {
    /// Returns `true` if the outcome represents a successful application.
    pub fn was_applied(&self) -> bool {
        matches!(self, Self::Applied | Self::AppliedWithConflicts(_))
    }
}
