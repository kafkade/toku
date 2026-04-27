CREATE TABLE reading_progress (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    session_id TEXT REFERENCES reading_sessions(id) ON DELETE SET NULL,
    progress_type TEXT NOT NULL DEFAULT 'page',
    value INTEGER NOT NULL,
    note TEXT,
    logged_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_reading_progress_book ON reading_progress(book_id);
CREATE INDEX idx_reading_progress_session ON reading_progress(session_id);
