-- Ebook files associated with books (multi-format per book).
-- Local-only: file binaries are never synced (see ROADMAP §6.4, Phase 7 cut line).
CREATE TABLE files (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id),
    path TEXT NOT NULL,
    format TEXT NOT NULL,              -- epub, pdf, mobi, azw3
    size_bytes INTEGER NOT NULL,
    checksum TEXT NOT NULL,            -- SHA-256, hex-encoded
    created_at TEXT NOT NULL,          -- ISO 8601
    updated_at TEXT NOT NULL           -- ISO 8601
);
CREATE INDEX idx_files_book ON files(book_id);
CREATE INDEX idx_files_checksum ON files(checksum);
