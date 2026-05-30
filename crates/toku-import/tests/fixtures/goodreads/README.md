# Goodreads CSV Export — Test Fixture

Sample Goodreads CSV export for testing `toku import goodreads`.

Exported via **My Books → Import/Export** on Goodreads.

## Column Reference

| Column | Type | Notes |
|---|---|---|
| `Book Id` | integer | Goodreads book ID, used for dedup on re-import |
| `Title` | string | May contain commas (field is then double-quoted) |
| `Author` | string | Primary author, first-last format |
| `Author l-f` | string | Primary author, last-first format (unused by importer) |
| `Additional Authors` | string | Comma-separated additional authors |
| `ISBN` | string | Wrapped as `="0441172717"` to prevent Excel number coercion |
| `ISBN13` | string | Wrapped as `="9780441172719"` — same quoting |
| `My Rating` | 0–5 | 0 means unrated; Toku converts to 0–10 scale |
| `Average Rating` | float | Community average (unused by importer) |
| `Publisher` | string | Publisher name (unused by importer) |
| `Binding` | string | Maps to BookFormat: `Kindle Edition`→Ebook, `Audiobook`→Audiobook, else Physical |
| `Number of Pages` | integer | May be empty |
| `Year Published` | integer | Edition year; fallback if Original Publication Year missing |
| `Original Publication Year` | integer | Preferred pub date; may be negative (BCE) |
| `Date Read` | `YYYY/MM/DD` | May be empty if unread |
| `Date Added` | `YYYY/MM/DD` | When user added book to Goodreads |
| `Bookshelves` | string | Comma-separated shelf names → imported as tags |
| `Bookshelves with positions` | string | Same shelves with `(#N)` position suffix (unused) |
| `Exclusive Shelf` | string | `read`, `currently-reading`, `to-read`, or custom |
| `My Review` | string | Free text; may contain escaped quotes (`""`) |
| `Spoiler` | string | Spoiler flag (unused) |
| `Private Notes` | string | Private notes (unused) |
| `Read Count` | integer | Times read (unused) |
| `Owned Copies` | integer | Owned copies (unused) |

## Edge Cases Covered in `export.csv`

| Row | Book | Edge Case |
|---|---|---|
| 1 | Dune | Standard read book, ISBN-10 + ISBN-13, rating 5, tags, review text |
| 2 | Neuromancer | `to-read` status, Kindle Edition → Ebook format, no Date Read |
| 3 | Project Hail Mary | `currently-reading`, rating 0 (unrated), no shelves/tags |
| 4 | Good Omens | **Comma in title** (quoted field), **multiple authors**, escaped quotes in review (`""`), high read count |
| 5 | Untitled Manuscript | **No ISBN**, no publisher, no pages, no binding — minimal data |
| 6 | The Art of War | **Negative publication year** (BCE), **3 authors** (2 additional), Audiobook format |
| 7 | Gödel's Proof | **Non-ASCII characters** in title (umlaut, apostrophe), no ISBN-13 |
| 8 | My Year of Rest... | **Non-standard exclusive shelf** (`favorites`) → becomes tag, Kindle format |
