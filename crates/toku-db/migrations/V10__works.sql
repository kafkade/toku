-- Works table: groups multiple editions (Books) of the same creative work.
CREATE TABLE works (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    original_language TEXT,
    first_published TEXT,
    created_at TEXT NOT NULL
);

-- Case-insensitive search by work title.
CREATE INDEX idx_works_title ON works(title COLLATE NOCASE);
