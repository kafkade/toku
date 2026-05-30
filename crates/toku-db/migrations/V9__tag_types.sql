-- Add tag_type column to tags table.
-- Tags are now unique by (name, tag_type) instead of just (name).
-- This allows the same name in different categories (e.g. "dark" as mood
-- and "dark" as a general genre tag).

CREATE TABLE tags_new (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL COLLATE NOCASE,
    tag_type TEXT NOT NULL DEFAULT 'general',
    created_at TEXT NOT NULL,
    UNIQUE(name, tag_type)
);

INSERT INTO tags_new (id, name, tag_type, created_at)
    SELECT id, name, 'general', created_at FROM tags;

DROP TABLE tags;

ALTER TABLE tags_new RENAME TO tags;
