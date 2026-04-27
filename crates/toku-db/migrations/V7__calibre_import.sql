ALTER TABLE books ADD COLUMN calibre_id INTEGER;
CREATE UNIQUE INDEX idx_books_calibre_id ON books(calibre_id) WHERE calibre_id IS NOT NULL;
