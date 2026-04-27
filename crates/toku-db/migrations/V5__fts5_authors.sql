-- Add denormalized search text column for FTS5 indexing.
-- Combines title, subtitle, description, and author names into one field.
ALTER TABLE books ADD COLUMN search_text TEXT;

-- Backfill search_text for existing books (title + subtitle + description).
-- Author names will be added by the application when update_search_text is called.
UPDATE books SET search_text = COALESCE(title, '')
    || CASE WHEN subtitle IS NOT NULL THEN ' ' || subtitle ELSE '' END
    || CASE WHEN description IS NOT NULL THEN ' ' || description ELSE '' END;

-- Drop old FTS5 table and triggers.
DROP TRIGGER IF EXISTS books_fts_insert;
DROP TRIGGER IF EXISTS books_fts_delete;
DROP TRIGGER IF EXISTS books_fts_update;
DROP TABLE IF EXISTS books_fts;

-- Recreate FTS5 with single search_text column.
CREATE VIRTUAL TABLE books_fts USING fts5(
    search_text,
    content='books',
    content_rowid='rowid'
);

-- Triggers to keep FTS5 in sync with books table.
CREATE TRIGGER books_fts_insert AFTER INSERT ON books BEGIN
    INSERT INTO books_fts(rowid, search_text) VALUES (new.rowid, new.search_text);
END;

CREATE TRIGGER books_fts_delete AFTER DELETE ON books BEGIN
    INSERT INTO books_fts(books_fts, rowid, search_text) VALUES ('delete', old.rowid, old.search_text);
END;

CREATE TRIGGER books_fts_update AFTER UPDATE ON books BEGIN
    INSERT INTO books_fts(books_fts, rowid, search_text) VALUES ('delete', old.rowid, old.search_text);
    INSERT INTO books_fts(rowid, search_text) VALUES (new.rowid, new.search_text);
END;

-- Rebuild FTS5 index from existing data.
INSERT INTO books_fts(books_fts) VALUES ('rebuild');
