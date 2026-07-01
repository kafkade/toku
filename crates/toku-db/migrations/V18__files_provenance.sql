-- Provenance for ebook file associations: record how a file record originated.
-- Free-text, matching metadata_provenance/import_logs conventions.
ALTER TABLE files ADD COLUMN source TEXT NOT NULL DEFAULT 'user';   -- user, calibre, goodreads, ...
ALTER TABLE files ADD COLUMN source_ref TEXT;                       -- optional external reference (import id / original path)
