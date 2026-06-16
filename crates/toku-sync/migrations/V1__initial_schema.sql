CREATE TABLE libraries (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL
);

CREATE TABLE devices (
  device_id TEXT PRIMARY KEY,
  library_id TEXT NOT NULL REFERENCES libraries(id),
  device_name TEXT NOT NULL,
  auth_token_hash TEXT NOT NULL,
  last_seen TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE ops (
  op_id TEXT PRIMARY KEY,
  library_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  hlc TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  op_type TEXT NOT NULL,
  payload TEXT NOT NULL,
  received_at TEXT NOT NULL
);

CREATE INDEX idx_ops_library_hlc ON ops(library_id, hlc);

CREATE TABLE cursors (
  device_id TEXT NOT NULL,
  cursor_type TEXT NOT NULL,
  op_id TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (device_id, cursor_type)
);
