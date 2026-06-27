-- Authenticated, Secret-Key-gated device enrollment (issue #120).
--
-- Builds on the account layer (V5): device enrollment is now gated behind an
-- authenticated account session (which already proves possession of the
-- password + Secret Key via SRP). This migration adds the device lifecycle
-- state needed for an optional trusted-device approval flow and an instance
-- toggle to enable it.
--
-- Additive and backward-compatible: existing device rows default to 'active',
-- so the legacy passwordless / library-SRP relay paths are undisturbed.

-- Device lifecycle status. 'pending' devices have been enrolled by an
-- authenticated account but await approval from an existing trusted device
-- before they are issued a session token; 'rejected' devices were denied.
ALTER TABLE devices ADD COLUMN status TEXT NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'pending', 'rejected'));

-- Optional device public key (X25519) uploaded at enrollment (ADR-010 step 4).
-- Nullable for backward compatibility and for relay-only deployments.
ALTER TABLE devices ADD COLUMN device_public_key TEXT;

-- When enabled, a newly enrolled device on a library that already has an active
-- device is held in 'pending' until an existing trusted device (the account
-- owner) approves it. Off by default (Immich-style opt-in).
ALTER TABLE instance_config
    ADD COLUMN device_approvals_required INTEGER NOT NULL DEFAULT 0;
