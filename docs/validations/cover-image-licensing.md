# Validation: Cover Image Licensing for Local Caching

**Status**: Validated
**Date**: 2026-05-29
**Issue**: [#19](https://github.com/kafkade/toku/issues/19)
**Affects**: `toku-meta` crate, cover image pipeline, future Google Books integration

## Summary

| Source | Local caching (on-demand) | Bulk pre-fetching | Attribution | Cover fallback viable? |
|--------|--------------------------|-------------------|-------------|----------------------|
| **Open Library** | ✅ Permitted | ❌ Prohibited | Courtesy (not required) | ✅ Yes — primary source |
| **Google Books** | ❌ Prohibited | ❌ Prohibited | Required ("Powered by Google" + links) | ❌ No — thumbnails cannot be stored locally |
| **User-provided** | ✅ N/A (user-directed) | ✅ N/A | N/A | ✅ Yes |
| **Calibre import** | ✅ User-directed local copy | ✅ User-directed | N/A | ✅ Yes |

## Open Library Covers API

### What is permitted

Open Library's general API guidelines explicitly encourage caching:

> "Please Do: **Cache responses whenever possible**"
> — [openlibrary.org/developers/api](https://openlibrary.org/developers/api)

Toku's use case — fetching a single cover image when a user runs `toku add --isbn` and
storing it locally content-addressed by SHA-256 — is consistent with this guidance. This is
user-initiated, on-demand, single-request behavior. Not crawling.

Open Library explicitly prioritizes open-source and mission-aligned projects:

> "We prioritize: **Open-source and mission-aligned projects**"
> — [openlibrary.org/developers/api](https://openlibrary.org/developers/api)

### What is NOT permitted

The Covers API documentation draws a clear line against bulk usage:

> "**Please, do not crawl our cover API.** If you do, we may decide to block your crawl."
> — [openlibrary.org/dev/docs/api/covers](https://openlibrary.org/dev/docs/api/covers)
>
> "**The covers API is intended for displaying covers on public facing websites and not
> for bulk download.**"
> — [openlibrary.org/dev/docs/api/covers](https://openlibrary.org/dev/docs/api/covers)

Rate limits apply: 100 requests per 5 minutes for ISBN-based cover lookups.

### Attribution

Attribution is framed as a courtesy, not a legal obligation:

> "A **courtesy** link back to Open Library is **appreciated**"
> — [openlibrary.org/dev/docs/api/covers](https://openlibrary.org/dev/docs/api/covers)

### Copyright status of cover images

The Internet Archive's licensing page clarifies:

> "The Internet Archive does not assert any new copyright or other proprietary rights over
> any of the material in the Open Library database."
> — [openlibrary.org/developers/licensing](https://openlibrary.org/developers/licensing)

This applies to the bibliographic database (metadata), not the cover images themselves.
Cover images are reproductions of copyrighted book jacket artwork owned by publishers and
designers. Open Library does not hold copyright to this artwork and cannot grant a license
for it. However, for a personal, offline-first book manager that caches covers locally for
the user's own private library display, this falls within established fair use patterns
(personal use, non-commercial, non-redistributive).

### Project policy for Open Library covers

Based on the reviewed terms, the project adopts this policy:

- ✅ **On-demand fetch**: Fetching a cover when a user adds a book by ISBN is permitted.
- ✅ **Local caching**: Storing the fetched cover on the user's local filesystem is permitted.
- ✅ **Offline display**: Displaying a locally cached cover without network is permitted.
- ❌ **Bulk crawling**: Programmatically fetching covers for many books without user action
  is not permitted. Any batch enrichment must be rate-limited and user-initiated.
- ❌ **Redistribution**: Redistributing cached cover images (e.g., bundling them in an app
  binary, serving them from a CDN) is not permitted.
- 🔄 **Courtesy attribution**: Include "Cover images from Open Library" in the README and
  About screen. Not legally required but good community citizenship.

## Google Books API

### Thumbnails CANNOT be cached locally

The Google APIs Terms of Service (Section 5e.1) explicitly prohibit permanent local storage:

> "you will not [...] **Scrape, build databases, or otherwise create permanent copies of
> such content, or keep cached copies longer than permitted by the cache header**"
> — [developers.google.com/terms](https://developers.google.com/terms) (Section 5e.1)

This prohibition is unambiguous and applies to all content returned by the API, including
book thumbnail images. Toku's offline-first architecture requires permanent local storage
of covers — this directly conflicts with the ToS.

### Metadata caching is also restricted

The same Section 5e.1 restriction applies to all API content, not just thumbnails. This
means:

- Caching raw Google Books API responses is limited to the duration specified by HTTP
  `Cache-Control` headers.
- Building a local database of Google Books metadata (title, author, etc.) may violate
  the "build databases" clause.
- Using Google Books for one-time field enrichment (fetch → merge into user's record →
  discard raw response) is likely the safest approach.

### API keys cannot be embedded in open-source projects

> "**Developer credentials may not be embedded in open source projects.**"
> — [developers.google.com/terms](https://developers.google.com/terms) (Section 4b.1)

This means Toku cannot ship with a bundled Google Books API key. Users would need to
supply their own key.

### Attribution requirements are heavy

The Google Books Branding Guidelines require:

- "Powered by Google" logo adjacent to any book results
- Prominent links to Google Books pages on every book result
- No result reordering or alteration

These requirements conflict with Toku's design principles (no external branding, offline
functionality, user data ownership).

### Project policy for Google Books

Based on the reviewed terms, the project adopts this policy:

- ❌ **Thumbnail caching**: Google Books thumbnails must NOT be downloaded and stored locally.
- ⚠️ **Metadata enrichment**: Google Books may be used for transient metadata enrichment
  (fetch once, extract fields, discard raw response). Raw API responses must not be cached
  beyond HTTP cache headers.
- ❌ **Bundled API key**: No Google Books API key may be shipped in the source code. Users
  must provide their own.
- ⚠️ **Deferred**: Given these restrictions, Google Books as a metadata fallback is deferred
  until the implications are fully evaluated. Open Library is sufficient as the primary and
  sole metadata source for the foreseeable future.

## Impact on Architecture

### Current state (no changes needed)

The existing `fetch_cover` function in `toku-meta/src/openlibrary.rs` fetches covers
on-demand when a user adds a book by ISBN. This is consistent with Open Library's terms:

1. User initiates `toku add --isbn <isbn>`
2. One HTTP request to `covers.openlibrary.org/b/isbn/{isbn}-L.jpg`
3. Image stored locally at `covers/{sha256_hash}.jpg`
4. No bulk fetching, no crawling, no redistribution

### Future Google Books integration

When Google Books is implemented as a metadata fallback:

- Use it for **text metadata only** (title, author, page count, description)
- Do **NOT** download or cache thumbnails from Google Books
- Do **NOT** cache raw API responses beyond HTTP cache headers
- Require users to supply their own API key via config
- If a book has no Open Library cover, leave the cover field empty rather than
  caching a Google Books thumbnail

### Calibre import covers

Calibre import copies cover images from the user's local Calibre library directory.
This is user-directed copying from a user-provided local source and is outside the scope
of API provider terms. No licensing concern.

## Sources Reviewed

| Source | URL |
|--------|-----|
| Open Library API Guidelines | <https://openlibrary.org/developers/api> |
| Open Library Covers API | <https://openlibrary.org/dev/docs/api/covers> |
| Open Library Licensing | <https://openlibrary.org/developers/licensing> |
| Google APIs Terms of Service | <https://developers.google.com/terms> |
| Google Books API Terms | <https://developers.google.com/books/terms> |
| Google Books Branding Guidelines | <https://developers.google.com/books/branding> |

> **Disclaimer**: This document reflects a good-faith review of publicly available provider
> terms as of May 2026. It is not legal advice. Terms may change; verify current terms
> before implementing new integrations.
