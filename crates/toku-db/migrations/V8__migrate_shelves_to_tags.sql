-- Migrate shelf memberships into tags.
-- For each shelf, create a tag with the same name (if not already present),
-- then copy book_shelves rows into book_tags.

INSERT OR IGNORE INTO tags (id, name, created_at)
SELECT id, name, created_at FROM shelves;

INSERT OR IGNORE INTO book_tags (book_id, tag_id)
SELECT bs.book_id, s.id
FROM book_shelves bs
JOIN shelves s ON s.id = bs.shelf_id;
