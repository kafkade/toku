-- Security-review hardening follow-ups (issue #160, threat model #125).
--
-- Additive and backward-compatible. Introduces:
--   1. An audit log for security-relevant events (F7).
--   2. An opaque key/value server-config table holding a random server secret
--      used to derive deterministic *phantom* SRP credentials for unknown or
--      disabled accounts, so the account challenge stage does not leak whether
--      an email exists (F5). The secret value is generated lazily in
--      application code because SQLite has no portable CSPRNG.

-- Append-only security audit trail. Written for failed logins, admin actions
-- (user enable/disable, registration toggle, device-approval toggle), and
-- device approve/reject decisions. Never holds secrets or plaintext content.
CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          TEXT    NOT NULL DEFAULT (datetime('now')),
    event_type  TEXT    NOT NULL,   -- e.g. 'login.failure', 'user.status', 'device.approval'
    actor       TEXT,               -- acting principal (user id / device id / 'anonymous')
    target      TEXT,               -- affected principal (user id / device id / email)
    outcome     TEXT    NOT NULL,   -- 'success' | 'failure'
    detail      TEXT,               -- free-form context (never secrets)
    ip          TEXT                -- client IP when known
);
CREATE INDEX idx_audit_log_ts ON audit_log(ts);
CREATE INDEX idx_audit_log_event ON audit_log(event_type);

-- Opaque server-side configuration (key/value). Currently stores a single
-- high-entropy 'server_secret' used only to derive phantom SRP credentials
-- (never leaves the server, never derives any user data).
CREATE TABLE server_config (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
