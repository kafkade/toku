-- Sync conflict tracking for note/review edits that collide across devices.
CREATE TABLE sync_conflicts (
    id TEXT PRIMARY KEY NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    field_name TEXT,
    local_value TEXT,
    remote_value TEXT,
    local_hlc TEXT NOT NULL,
    remote_hlc TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0,
    resolved_at TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_sync_conflicts_entity ON sync_conflicts(entity_type, entity_id);
CREATE UNIQUE INDEX idx_sync_conflicts_dedup ON sync_conflicts(entity_type, entity_id, field_name, remote_hlc);

-- Per-field HLC for sync LWW comparison (separate from source_date which uses RFC3339).
ALTER TABLE metadata_provenance ADD COLUMN sync_hlc TEXT;

-- Soft delete support for sync tombstones.
ALTER TABLE books ADD COLUMN deleted_at TEXT;
