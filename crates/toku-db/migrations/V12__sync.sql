-- Sync operations: local staging for push/pull sync.
-- Each mutation appends an op that can be pushed to a sync server.
CREATE TABLE sync_ops (
    op_id TEXT PRIMARY KEY NOT NULL,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL,
    entity_type TEXT NOT NULL CHECK (entity_type IN ('book', 'session', 'progress', 'tag', 'note', 'review', 'setting', 'device')),
    entity_id TEXT NOT NULL,
    op_type TEXT NOT NULL CHECK (op_type IN ('create', 'update', 'delete')),
    fields_json TEXT,
    checksum TEXT NOT NULL,
    pushed_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_sync_ops_pushed ON sync_ops(pushed_at);
CREATE INDEX idx_sync_ops_entity ON sync_ops(entity_type, entity_id);
CREATE INDEX idx_sync_ops_hlc ON sync_ops(hlc);

-- Sync cursors: track push/pull progress.
CREATE TABLE sync_cursors (
    key TEXT PRIMARY KEY NOT NULL CHECK (key IN ('push_cursor', 'pull_cursor')),
    value TEXT NOT NULL
);

-- Device identity: identifies this installation for sync.
CREATE TABLE sync_device (
    device_id TEXT PRIMARY KEY NOT NULL,
    device_name TEXT NOT NULL,
    created_at TEXT NOT NULL
);
