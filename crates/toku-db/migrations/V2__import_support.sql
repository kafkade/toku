-- Add goodreads_id column to books for import dedup.
ALTER TABLE books ADD COLUMN goodreads_id TEXT;
CREATE UNIQUE INDEX idx_books_goodreads_id ON books(goodreads_id) WHERE goodreads_id IS NOT NULL;

-- Import logs table to track every import operation.
CREATE TABLE import_logs (
    id TEXT PRIMARY KEY NOT NULL,
    source TEXT NOT NULL,
    file_path TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    total_rows INTEGER NOT NULL DEFAULT 0,
    imported INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0
);

-- Track which books came from which import for rollback.
CREATE TABLE import_books (
    import_id TEXT NOT NULL REFERENCES import_logs(id) ON DELETE CASCADE,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    PRIMARY KEY (import_id, book_id)
);
