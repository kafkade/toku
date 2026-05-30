# Validation: StoryGraph Export Availability and Format

**Status**: Validated
**Date**: 2026-05-29
**Issue**: [#17](https://github.com/kafkade/toku/issues/17)
**Affects**: `toku-import` crate (Phase 3 StoryGraph importer)

## Summary

StoryGraph offers a **free, built-in CSV export** of the user's entire library. The export
produces a 23-column comma-separated UTF-8 file with rich reading-experience data that
Goodreads has no equivalent for (moods, pace, character questions, content warnings,
quarter-star ratings, full re-read date ranges, and native DNF status). There is **no
public API** — CSV export is the only viable data access method.

**Verdict**: ✅ StoryGraph export is available and well-structured. An importer is feasible.

## Export Location and Process

1. Log in to `app.thestorygraph.com`
2. Click avatar → Profile
3. Scroll to "Export your data" → click export
4. Download the CSV file (direct download or emailed)

> **Note**: The exported file may be named `goodreads_library_export_<username>.csv` — a
> legacy naming artifact from StoryGraph's origins. Detect StoryGraph exports by the
> presence of StoryGraph-specific columns (`Moods`, `Pace`, `Character- or Plot-Driven?`).

## File Format

| Property | Value |
|----------|-------|
| Delimiter | Comma (`,`) |
| Encoding | UTF-8 (with optional BOM — use `utf-8-sig` decoding) |
| Line endings | Mixed (`\r\n` and `\n` both observed) |
| Header row | Yes — row 1 contains column names |
| Quote character | Double-quote (`"`) |
| Escaped quotes | `""` (RFC 4180 standard) |
| ISBN wrapping | Clean — no `=""` Excel-style wrapping (unlike Goodreads) |
| Null representation | Empty string `""` |

## Complete Column Reference (23 Columns)

### Book metadata

| # | Column Name | Type | Empty Rate | Notes |
|---|-------------|------|------------|-------|
| 1 | `Title` | String | 0% | Clean title — no series info embedded (unlike Goodreads) |
| 2 | `Authors` | String | 0% | First Last format. Multiple authors comma-separated |
| 3 | `Contributors` | String | ~95% | Narrators, translators with role: `"Name (Role)"` |
| 4 | `ISBN/UID` | String | ~20% | Mixed: ISBN-13, ISBN-10, ASIN, or empty |
| 5 | `Format` | Enum | ~2% | `"digital"`, `"paperback"`, `"hardcover"`, `"audio"` |
| 23 | `Owned?` | Enum | 0% | `"Yes"` or `"No"` |

**Not exported** (must be enriched via Open Library or other sources):

- Page count
- Publisher
- Publication year
- Cover image URL
- Series name and number

### Reading tracking

| # | Column Name | Type | Empty Rate | Notes |
|---|-------------|------|------------|-------|
| 6 | `Read Status` | Enum | 0% | `"read"`, `"to-read"`, `"currently-reading"`, `"did-not-finish"` |
| 7 | `Date Added` | Date | 0% | `YYYY/MM/DD` format |
| 8 | `Last Date Read` | Date | ~40% | Variable precision: `YYYY/MM/DD`, `YYYY/MM`, or `YYYY` |
| 9 | `Dates Read` | Complex | ~40% | Multi-session field (see parsing spec below) |
| 10 | `Read Count` | Integer | 0% | `0` for unread books |

### Rating

| # | Column Name | Type | Empty Rate | Notes |
|---|-------------|------|------------|-------|
| 18 | `Star Rating` | Decimal | ~35% | Quarter-star: 1.0 to 5.0 in 0.25 increments. Empty = unrated |

### Moods, pace, and reading experience

| # | Column Name | Type | Empty Rate | Values |
|---|-------------|------|------------|--------|
| 11 | `Moods` | Comma-separated | ~50% | 13 canonical values (see list below) |
| 12 | `Pace` | Enum | ~50% | `"slow"`, `"medium"`, `"fast"` |
| 13 | `Character- or Plot-Driven?` | Enum | ~55% | `"Character"`, `"Plot"`, `"A mix"` |
| 14 | `Strong Character Development?` | Ternary | ~55% | `"Yes"`, `"No"`, `"It's complicated"` |
| 15 | `Loveable Characters?` | Ternary | ~55% | `"Yes"`, `"No"`, `"It's complicated"` |
| 16 | `Diverse Characters?` | Ternary | ~60% | `"Yes"`, `"No"`, `"It's complicated"` |
| 17 | `Flawed Characters?` | Ternary | ~60% | `"Yes"`, `"No"`, `"It's complicated"` |

### Content warnings, reviews, tags

| # | Column Name | Type | Empty Rate | Notes |
|---|-------------|------|------------|-------|
| 19 | `Review` | HTML string | ~75% | Contains `<div>`, `<br>`, `<spoiler>` tags |
| 20 | `Content Warnings` | Structured | ~99% | `"Severity: Type; Severity: Type;"` |
| 21 | `Content Warning Description` | String | ~99% | Free-text user notes |
| 22 | `Tags` | Comma-separated | ~99% | User-defined, case-inconsistent |

## Parsing Specifications

### Dates Read (complex multi-session field)

This field encodes one or more reading sessions with variable date precision:

```text
Single month:          "2024/09"
Full date range:       "2025/12/11-2025/12/14"
Same-day read:         "2025/07/08-2025/07/08"
Multiple sessions:     "2025/07/08-2025/07/08, 2024"
Complex multi-session: "2025/02/13-2025/03/15, 2024/09-2024/09"
```

**Parsing algorithm**:

1. Split on `", "` to get individual sessions
2. Split each session on `"-"` to get start and end dates
3. Parse each date segment with variable precision:
   - `YYYY/MM/DD` → full date
   - `YYYY/MM` → first of month
   - `YYYY` → first of year

### Moods taxonomy (13 canonical values)

```text
adventurous, challenging, dark, emotional, funny, hopeful,
inspiring, lighthearted, mysterious, reflective, relaxing, sad, tense
```

Parse by splitting on `","` and trimming whitespace. Normalize to lowercase.

### Content Warnings format

```text
"Graphic: Sexual content; Moderate: Cancer; Minor: Death of parent;"
```

Severity levels: `Graphic`, `Moderate`, `Minor`. Parse by splitting on `";"`, then
splitting each part on `":"`.

### Star Rating conversion

StoryGraph uses quarter-star increments (0.25) on a 1–5 scale.
Toku uses a 0–10 integer scale.

Conversion: `toku_rating = round(storygraph_star * 2)`

| StoryGraph | Toku | Display |
|------------|------|---------|
| 1.0 | 2 | ★☆☆☆☆ |
| 2.5 | 5 | ★★½☆☆ |
| 3.75 | 8 | ★★★¾☆ |
| 4.5 | 9 | ★★★★½ |
| 5.0 | 10 | ★★★★★ |

### ISBN/UID field detection

The `ISBN/UID` column contains mixed identifier types:

| Pattern | Type | Action |
|---------|------|--------|
| 13 digits | ISBN-13 | Store as `isbn13` identifier |
| 10 chars matching `[0-9Xx]{10}` | ISBN-10 | Store as `isbn10`, convert to ISBN-13 |
| Starts with `B0` + 8 alphanumeric | ASIN | Store as `asin` identifier |
| Empty | No identifier | Match by title + author |

### Read Status mapping

| StoryGraph | Toku `ReadingStatus` |
|------------|---------------------|
| `read` | `Read` |
| `to-read` | `WantToRead` |
| `currently-reading` | `Reading` |
| `did-not-finish` | `Abandoned` |
| `paused` (rare) | `Reading` (treat as on-hold/reading) |

### Format mapping

| StoryGraph | Toku `BookFormat` |
|------------|-------------------|
| `paperback` | `Physical` |
| `hardcover` | `Physical` |
| `digital` | `Ebook` |
| `audio` | `Audiobook` |

## Import Fidelity Assessment

### High fidelity (direct mapping)

- ✅ Title, Authors → `books.title`, `book_contributors`
- ✅ Read Status → `ReadingStatus` (native DNF support maps to `Abandoned`)
- ✅ Star Rating → `books.rating` (quarter-star → 0-10 integer, lossless)
- ✅ Date Added → `books.created_at`
- ✅ Dates Read → `reading_sessions` (start/end per session, re-reads preserved)
- ✅ Read Count → validates against number of `reading_sessions`
- ✅ Moods → `tags` with `tag_type = 'mood'` (requires #42)
- ✅ Pace → `tags` with `tag_type = 'pace'` (requires #42)
- ✅ Format → `BookFormat` (direct mapping)
- ✅ ISBN/UID → `identifiers` table (ISBN-13, ISBN-10, or ASIN)
- ✅ Tags → `tags` with `tag_type = 'general'`
- ✅ Review → `reviews.text` (strip HTML, preserve `<spoiler>` as markdown)

### Medium fidelity (enrichment needed)

- ⚠️ Page count — not exported, must enrich via Open Library ISBN lookup
- ⚠️ Series — not in export, must enrich externally
- ⚠️ Publisher, publication year — not exported
- ⚠️ Cover images — not exported, fetch via Open Library

### StoryGraph-specific data (extended mapping)

These fields are unique to StoryGraph and have no Goodreads equivalent:

- 🆕 Character- or Plot-Driven? → `custom_fields` or new tag type
- 🆕 Strong/Loveable/Diverse/Flawed Characters → `custom_fields` or tag type
- 🆕 Content Warnings → `tags` with `tag_type = 'content_warning'` (requires #42)
- 🆕 Content Warning Description → linked note or `custom_fields`
- 🆕 Contributors (narrators) → `book_contributors` with `role = 'narrator'`
- 🆕 Owned? → `custom_fields` or future `owned` column

## Known Quirks

1. **No series data in titles** — Unlike Goodreads which embeds `"(Series, #N)"` in titles,
   StoryGraph exports clean titles. Series must be enriched externally.

2. **Variable date precision** — Dates can be year-only, month/year, or full dates. Parser
   must handle all three gracefully.

3. **HTML in reviews** — Reviews contain `<div>`, `<br>`, `&nbsp;`, and custom `<spoiler>`
   tags. Strip HTML on import, convert `<spoiler>` to markdown spoiler syntax.

4. **Mixed ISBN/UID field** — Single column for ISBN-13, ISBN-10, ASIN, and unknown IDs.
   Requires detection heuristic.

5. **Author comma ambiguity** — Multi-author entries use commas as separators, same as the
   CSV delimiter. Standard CSV quoting handles this, but the importer must split the
   `Authors` field on `","` after CSV parsing.

6. **Tags are rarely used** — ~99% empty. Deduplicate case-insensitively when present.

7. **`paused` status** — Rare but observed. Map to `Reading` (or `OnHold` if supported).

8. **Legacy filename** — Export may be named `goodreads_library_export_*.csv`. Detect
   StoryGraph by presence of `Moods` or `Pace` columns in the header row.

## No Public API

There is no public StoryGraph API. CSV export is the only viable data access method.
Web scraping would violate the project's non-goals (Section 3.5: "No scraping").

## Sources

Research verified against multiple independent implementations:

- Real 338-book export analysis (field frequency, value distributions)
- TypeScript parser: `nperez0111/bookhive` (`src/utils/csv.ts`)
- Python parser: `beforetheshoes/theseedbed` (`storygraph_parser.py`, 96 tests)
- Python cleaner: `Runekeon/book_tracking_stuff` (`story_graph_export_cleaner.py`)
- JS parser: `mmsge/read2listen` (`src/utils/storygraph.js`)
