-- Notes attached to a book (personal annotations).
CREATE TABLE notes (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id),
    content TEXT NOT NULL,
    deleted_at TEXT,
    deleted_by_device TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_notes_book ON notes(book_id);

-- Reviews (a user's review of a book — one per book).
CREATE TABLE reviews (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id),
    content TEXT,
    rating INTEGER,
    deleted_at TEXT,
    deleted_by_device TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_reviews_book ON reviews(book_id);

-- User settings (key-value, synced via LWW per key).
CREATE TABLE user_settings (
    id TEXT PRIMARY KEY NOT NULL,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    sync_hlc TEXT,
    updated_at TEXT NOT NULL
);

-- Per-entity HLC tracking for sync merge of notes, reviews, and other
-- non-book entities. Tracks which device last wrote each field.
CREATE TABLE sync_entity_hlc (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    field_name TEXT NOT NULL,
    sync_hlc TEXT NOT NULL,
    device_id TEXT,
    PRIMARY KEY (entity_type, entity_id, field_name)
);
