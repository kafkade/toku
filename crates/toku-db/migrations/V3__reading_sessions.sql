CREATE TABLE reading_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    start_page INTEGER,
    end_page INTEGER,
    rating INTEGER CHECK (rating IS NULL OR (rating >= 0 AND rating <= 10)),
    notes TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_reading_sessions_book ON reading_sessions(book_id);
