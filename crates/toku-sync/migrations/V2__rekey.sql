-- Add salt column to libraries for per-library encryption salt.
ALTER TABLE libraries ADD COLUMN salt TEXT;

-- Rekey lock to prevent concurrent pushes during re-keying.
ALTER TABLE libraries ADD COLUMN rekey_in_progress INTEGER NOT NULL DEFAULT 0;
