CREATE TABLE books (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    subtitle TEXT,
    description TEXT,
    page_count INTEGER,
    pub_date TEXT,
    language TEXT,
    format TEXT NOT NULL DEFAULT 'physical',
    duration_minutes INTEGER,
    cover_hash TEXT,
    work_id TEXT,
    status TEXT NOT NULL DEFAULT 'want-to-read',
    rating INTEGER CHECK (rating IS NULL OR (rating >= 0 AND rating <= 10)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE authors (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    sort_name TEXT
);

CREATE TABLE book_authors (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'author',
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (book_id, author_id, role)
);

CREATE TABLE isbns (
    isbn TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE series (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    total_books INTEGER
);

CREATE TABLE book_series (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    series_id TEXT NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    position TEXT,
    PRIMARY KEY (book_id, series_id)
);

CREATE TABLE metadata_provenance (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    field_name TEXT NOT NULL,
    source TEXT NOT NULL,
    source_date TEXT NOT NULL,
    is_user_override INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (book_id, field_name)
);

-- FTS5 external-content table for full-text search.
-- Kept in sync via triggers below.
CREATE VIRTUAL TABLE books_fts USING fts5(
    title,
    subtitle,
    description,
    content='books',
    content_rowid='rowid'
);

-- Triggers to keep FTS5 in sync with books table.
CREATE TRIGGER books_fts_insert AFTER INSERT ON books BEGIN
    INSERT INTO books_fts(rowid, title, subtitle, description)
    VALUES (new.rowid, new.title, new.subtitle, new.description);
END;

CREATE TRIGGER books_fts_delete AFTER DELETE ON books BEGIN
    INSERT INTO books_fts(books_fts, rowid, title, subtitle, description)
    VALUES ('delete', old.rowid, old.title, old.subtitle, old.description);
END;

CREATE TRIGGER books_fts_update AFTER UPDATE ON books BEGIN
    INSERT INTO books_fts(books_fts, rowid, title, subtitle, description)
    VALUES ('delete', old.rowid, old.title, old.subtitle, old.description);
    INSERT INTO books_fts(rowid, title, subtitle, description)
    VALUES (new.rowid, new.title, new.subtitle, new.description);
END;

-- Indexes for common queries.
CREATE INDEX idx_books_status ON books(status);
CREATE INDEX idx_books_title ON books(title);
CREATE INDEX idx_books_work_id ON books(work_id);
CREATE INDEX idx_book_authors_author ON book_authors(author_id);
CREATE INDEX idx_isbns_book ON isbns(book_id);
CREATE INDEX idx_book_series_series ON book_series(series_id);
