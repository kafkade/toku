-- Track whether this device has completed its initial sync bootstrap.
--
-- Set once the device has either backfilled its existing state (first opt-in
-- on signup / fresh-library enroll) or restored a prior library from the
-- server (new-device enroll / deferred post-approval login). This is a
-- device-local marker (never synced) so routine logins don't repeatedly
-- re-run bootstrap. See ADR-013 (D3).
ALTER TABLE sync_device ADD COLUMN bootstrapped_at TEXT;
