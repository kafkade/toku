-- Snapshot storage for compaction and new-device bootstrap.
CREATE TABLE snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    library_id TEXT NOT NULL REFERENCES libraries(id),
    snapshot_json TEXT NOT NULL,
    hlc_at_snapshot TEXT NOT NULL,
    created_by_device TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_snapshots_library ON snapshots(library_id, created_at DESC);
