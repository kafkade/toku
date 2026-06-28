-- Relay → account model migration support (issue #126, part of #114).
--
-- The account/user layer (V5) added nullable ownership columns but nothing
-- linked pre-existing relay libraries/devices to an account, and there was no
-- way to lock out pre-account clients once an instance migrated. This migration
-- adds the supporting structure. Backfill of orphan libraries/devices to the
-- first admin happens at runtime during the first `account_signup` (or via
-- `toku sync migrate`); refinery cannot manufacture SRP credentials here.
--
-- Additive, idempotent, and backward-compatible: defaults keep legacy relay
-- behaviour intact until a `migrate` is performed.

-- Speed up orphan-library lookups during backfill and ownership checks.
CREATE INDEX IF NOT EXISTS idx_libraries_user ON libraries(user_id);
CREATE INDEX IF NOT EXISTS idx_devices_user ON devices(user_id);

-- Wire-protocol versioning. `protocol_version` advertises what the server
-- speaks; `min_protocol` is the lowest version a client may use. Protocol 1 is
-- the legacy relay (library_id + single passphrase, unauthenticated register);
-- protocol 2 is the account/user model. `min_protocol` stays at 1 until an
-- admin migrates the instance, after which pre-account clients are rejected
-- with HTTP 426 (Upgrade Required).
ALTER TABLE instance_config ADD COLUMN protocol_version INTEGER NOT NULL DEFAULT 2;
ALTER TABLE instance_config ADD COLUMN min_protocol INTEGER NOT NULL DEFAULT 1;
