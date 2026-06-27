-- User accounts, admin roles, and multi-user schema (issue #119).
--
-- Additive and backward-compatible: the existing library-level SRP relay
-- (V4) is left untouched. This migration introduces *users* as the SRP
-- principal for account-level authentication, an instance-level registration
-- gate, and ownership links from libraries/devices to users.

-- Real user accounts. The server stores only the SRP verifier + salt (it never
-- sees the password or Secret Key — see #117) and opaque wrapped key material
-- (see #116). It cannot derive any plaintext data from these columns.
CREATE TABLE users (
    id                   TEXT    PRIMARY KEY,            -- uuid v7
    email                TEXT    NOT NULL UNIQUE,        -- account handle / login
    srp_salt             TEXT    NOT NULL,               -- hex-encoded SRP salt
    srp_verifier         TEXT    NOT NULL,               -- hex-encoded SRP verifier (g^x mod N)
    wrapped_private_key  TEXT,                           -- opaque wrapped account private key (#116)
    account_public_key   TEXT,                           -- account public key (#116)
    kdf_params           TEXT,                           -- versioned KDF params blob (#116)
    role                 TEXT    NOT NULL DEFAULT 'user'
                                 CHECK (role IN ('admin', 'user')),
    status               TEXT    NOT NULL DEFAULT 'active'
                                 CHECK (status IN ('active', 'disabled')),
    failed_attempts      INTEGER NOT NULL DEFAULT 0,
    locked_until         TEXT,                           -- ISO-8601 datetime; NULL = not locked
    created_at           TEXT    NOT NULL
);

-- Instance-wide configuration. Single row (id = 1). Self-registration is
-- CLOSED by default — the instance is invite/admin-gated like Immich. The
-- first account created on a fresh instance bootstraps as the admin regardless
-- of this flag.
CREATE TABLE instance_config (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    registration_open  INTEGER NOT NULL DEFAULT 0,       -- 0 = closed, 1 = open
    created_at         TEXT    NOT NULL,
    updated_at         TEXT    NOT NULL
);
INSERT OR IGNORE INTO instance_config (id, registration_open, created_at, updated_at)
VALUES (1, 0, datetime('now'), datetime('now'));

-- Ephemeral SRP server challenges for *user* logins (single-use, 5-min TTL).
-- Kept separate from the library-scoped `srp_challenges` so the existing relay
-- auth path is undisturbed.
CREATE TABLE user_srp_challenges (
    challenge_id             TEXT PRIMARY KEY,
    user_id                  TEXT NOT NULL REFERENCES users(id),
    server_ephemeral_secret  TEXT NOT NULL,  -- hex-encoded b
    client_public_a          TEXT NOT NULL,  -- hex-encoded A (needed for verify)
    created_at               TEXT NOT NULL
);
CREATE INDEX idx_user_srp_challenges_created ON user_srp_challenges(created_at);

-- Short-lived session tokens issued after successful user SRP verification.
-- Distinct from the device-scoped `sessions` table; user sessions authenticate
-- account/admin endpoints, not device sync traffic.
CREATE TABLE user_sessions (
    session_token_hash  TEXT    PRIMARY KEY,
    user_id             TEXT    NOT NULL REFERENCES users(id),
    expires_at          TEXT    NOT NULL,   -- ISO-8601 UTC datetime
    created_at          TEXT    NOT NULL
);
CREATE INDEX idx_user_sessions_expires ON user_sessions(expires_at);

-- Ownership links. Nullable for backward compatibility: pre-existing libraries
-- and devices registered under the open relay model remain valid (unowned).
-- New writes by an authenticated user stamp ownership.
ALTER TABLE libraries ADD COLUMN user_id TEXT REFERENCES users(id);
ALTER TABLE devices ADD COLUMN user_id TEXT REFERENCES users(id);
