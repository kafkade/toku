# Validation: Open Library API Rate Limits and Data Quality

**Status**: Validated
**Date**: 2026-05-30
**Issue**: [#15](https://github.com/kafkade/toku/issues/15)
**Affects**: `toku-meta` crate, ISBN lookup, search, cover fetching

## Summary

Open Library is a viable primary metadata source for Toku. Coverage is
strong for English-language books and popular international editions.
Key gaps exist in author data at the edition level and Japanese/CJK
editions.

## Rate Limits

| Endpoint | Stated limit | Observed behavior |
|---|---|---|
| Search API (`/search.json`) | 100 req/5 min | No throttling observed at low volume |
| ISBN API (`/isbn/{isbn}.json`) | 100 req/5 min | No throttling at 5 sequential requests |
| Covers API (`/b/isbn/{isbn}-L.jpg`) | 100 req/5 min (ISBN-based) | No throttling at low volume |

The stated rate limit of 100 requests per 5 minutes is generous for a
personal book manager. Toku's usage pattern (single-book lookups
initiated by the user) will never approach this limit under normal use.

**Recommendation**: No rate-limiting code needed in Toku. Add a simple
backoff-and-retry for transient 429 responses as a safety net.

## Response Times

Measured from Windows desktop over residential internet (sequential
requests, no parallelism):

| Query type | Avg | Min | Max | Notes |
|---|---|---|---|---|
| ISBN lookup | ~2.9s | 2.1s | 4.2s | Includes 1–2 author resolution sub-requests |
| Search query | ~2.5s | 2.2s | 3.9s | Single request, no sub-queries |
| Cover download (L-size) | ~0.6s | 0.4s | 0.8s | Served from CDN, faster than API |

ISBN lookups are slow because each requires a second HTTP request per
author to resolve `/authors/{key}.json`. A book with 3 authors makes 4
total requests.

**Recommendation**: Consider fetching author names from the work record
(`/works/{key}.json`) instead, which includes all authors in one
response. This would reduce ISBN lookups to 2 requests regardless of
author count.

## ISBN Lookup Coverage

### English bestsellers — ✅ Excellent

| Book | ISBN | Title | Authors | Pages | Cover |
|---|---|---|---|---|---|
| Dune | 9780441172719 | ✅ Dune | ✅ Frank Herbert | ✅ 535 | ✅ Yes |
| Harry Potter | 9780590353427 | ✅ | ⚠️ Missing | ❌ None | ✅ Yes |
| The Martian | 9780553418026 | ✅ | ⚠️ Missing | ✅ 388 | — |
| To Kill a Mockingbird | 9780061120084 | ✅ | ✅ Harper Lee | ✅ 323 | — |

### Non-English titles — ⚠️ Mixed

| Book | ISBN | Title | Authors | Language | Notes |
|---|---|---|---|---|---|
| Cien años de soledad (Spanish) | 9788497592208 | ✅ | ✅ García Márquez | ✅ spa | Good coverage |
| Norwegian Wood (Japanese) | 9784062749688 | ⚠️ Wrong title | — | ✅ jpn | Returns different book |

**Finding**: Spanish editions have good coverage. Japanese ISBNs may
resolve to incorrect editions. CJK coverage is unreliable.

### Old editions (pre-1970) — ✅ Good

| Book | ISBN | Title | Authors | Pub Date |
|---|---|---|---|---|
| To Kill a Mockingbird | 9780061120084 | ✅ | ✅ Harper Lee | 2006 (reprint) |
| 1984 | 9780451524935 | ✅ Nineteen Eighty-Four | ⚠️ Missing | 1993? |

**Finding**: Classic titles are well-covered but some editions lack
author links. The `pub_date` field reflects the edition date, not the
original publication date.

### Self-published — ✅ Adequate

The Martian (originally self-published, later picked up by Crown) is
present with correct metadata. Self-published books with ISBNs are
generally findable; those without ISBNs require search.

## Known Data Gaps

### 1. Missing authors at edition level (HIGH impact)

Some editions return an empty `authors` array even when the work record
has authors. This affects ~30% of tested ISBNs including major titles
(Harry Potter, 1984, The Martian).

**Root cause**: The edition record references authors via `/authors/{key}`
but some editions don't include these references. The author data exists
at the work level (`/works/{key}/authors.json`).

**Mitigation**: Fall back to the work record for author names when the
edition record has none. Requires an additional API request to
`/works/{key}.json`.

### 2. Missing page counts (MEDIUM impact)

Harry Potter (9780590353427) returned no `number_of_pages`. Some
editions simply don't have this field populated.

**Mitigation**: Show "unknown" in CLI. Allow user to fill in manually
via `toku edit --pages`.

### 3. Missing descriptions (LOW impact)

Tao Te Ching (9780679724346) returned no description. Many older or
niche editions lack descriptions.

**Mitigation**: Acceptable — descriptions are optional display data.

### 4. Japanese/CJK ISBN resolution (LOW impact for MVP)

ISBN 9784062749688 (Norwegian Wood, Japanese) returned a different book
entirely. CJK editions may have data quality issues in Open Library.

**Mitigation**: Acceptable for MVP. Japanese users can add books
manually. Consider adding a "confirm metadata" prompt after ISBN lookup.

### 5. Publication date ambiguity (LOW impact)

`pub_date` is a free-text string (e.g., "1993?", "June 1987", "2006").
Not always a clean year. The `Original Publication Year` from Goodreads
is more reliable for original dates.

**Mitigation**: Store as-is; parse best-effort for display.

## Cover Image Quality

| ISBN | Book | File Size | Quality |
|---|---|---|---|
| 9780441172719 | Dune | 48 KB | Good (L-size) |
| 9780590353427 | Harry Potter | 70 KB | Good (L-size) |
| 9799999999999 | (bogus) | — | Correctly returns no cover |

**Findings**:

- L-size covers (`-L.jpg`) are 48–70 KB, suitable for display
- The `?default=false` parameter correctly returns 404 for missing covers
- Placeholder detection (< 1000 bytes) works as implemented
- Cover CDN is faster than the API (~400–800ms vs ~2–3s)

## Recommendations for `toku-meta`

1. **Author fallback**: When edition authors are empty, fetch the work
   record to get author names. This is the highest-impact improvement.
2. **No rate limiting needed**: Usage pattern is well within limits.
   Add retry-with-backoff for transient errors only.
3. **Cache API responses**: Store raw metadata locally to avoid repeat
   lookups for the same ISBN. Already planned per API guidelines.
4. **User agent**: Update from `toku/0.1.0` to current version.

## Integration Tests

16 validation tests added in
`crates/toku-meta/tests/openlibrary_validation.rs`:

- 9 ISBN lookup tests (bestsellers, non-English, old editions,
  self-published, not-found, sparse data)
- 3 search tests (title, author, non-English)
- 3 cover tests (available, not available, quality check)
- 1 response time benchmark (5 sequential ISBNs)

Run with: `cargo test -p toku-meta -- --ignored`

All tests are `#[ignore]` to avoid hitting the network in CI.
