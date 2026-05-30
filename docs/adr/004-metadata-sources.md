# ADR-004: Metadata Sources — Open Library Primary, Google Books Fallback

**Status**: Accepted
**Date**: 2026-04-26
**Decision**: Use Open Library as the primary metadata source. Google Books as fallback.
User edits always take precedence over fetched data.

## Context

When a user adds a book by ISBN, the app should fetch metadata (title, author, page
count, description, cover image) to avoid manual entry. The metadata source must be free,
reliable, and compatible with open-source use.

## Decision

- **Primary**: Open Library REST API (`openlibrary.org/isbn/{isbn}.json`).
- **Fallback**: Google Books API (free tier, 1,000 req/day without key).
- **Cover images**: Open Library Covers API, Google Books thumbnails as fallback.
- **Merge strategy**: Query Open Library first. If no result or missing fields, query
  Google Books. Fill empty fields only — never overwrite existing data.
- **User edits always win**: Once a user modifies a field, auto-enrichment marks it as
  `user_override` and will not overwrite it.
- **Caching**: API responses cached locally for 30 days to reduce API calls and enable
  offline re-enrichment.

## Rationale

- Open Library is free, requires no API key, and is community-maintained.
- Google Books has broader coverage (especially recent releases and non-English titles)
  but has rate limits.
- Open Library is compatible with open-source use for on-demand caching. Google Books
  has significant ToS restrictions — see cover image licensing below.
- The "user wins" rule prevents data corruption from bad metadata sources.

## Cover Image Licensing — Validated (2026-05-29)

See [docs/validations/cover-image-licensing.md](../validations/cover-image-licensing.md)
for the full validation report.

**Open Library covers**: On-demand local caching is permitted. Toku fetches one cover per
user-initiated ISBN add and stores it locally. This is consistent with OL's API guidelines.
Bulk crawling is prohibited. Courtesy attribution is appreciated but not required.

**Google Books thumbnails**: Local caching is **prohibited** by the Google APIs ToS
(Section 5e.1 — no permanent copies, no caching beyond HTTP cache headers). Google Books
may only be used for transient text metadata enrichment, not cover images. API keys also
cannot be embedded in open-source projects (Section 4b.1).

## Alternatives Considered

| Source | Rejected Because |
|--------|-----------------|
| ISBNdb | Paid — violates $0 budget constraint |
| WorldCat | API terms unclear for open-source `[Validation Required]` |
| BookBrainz | Sparse coverage, growing but not ready as primary |
| Hardcover | API availability uncertain `[Validation Required]` |

## Open Validations

- [ ] Open Library rate limits (stated 100 req/min — verify in practice)
- [x] Google Books ToS for open-source caching — **Validated 2026-05-29**: Significant
  restrictions apply. Permanent caching prohibited. API keys cannot be embedded in
  open-source projects. See [cover-image-licensing.md](../validations/cover-image-licensing.md).
- [x] Cover image licensing for local caching — **Validated 2026-05-29**: Open Library
  on-demand caching permitted. Google Books thumbnail caching prohibited.
  See [cover-image-licensing.md](../validations/cover-image-licensing.md).
