# Validation: Google Books API Terms for Open-Source Use

**Status**: Validated
**Date**: 2026-05-30
**Issue**: [#16](https://github.com/kafkade/toku/issues/16)
**Affects**: `toku-meta` crate, metadata fallback strategy (ADR-004)

## Summary

| Question | Answer |
|---|---|
| Can an open-source app cache metadata locally? | ⚠️ Only transiently — permanent copies prohibited (Section 5e.1) |
| Free tier quota? | 1,000 req/day per API key (default); no reliable unauthenticated access |
| API key in binary? | ❌ Prohibited — "Developer credentials may not be embedded in open source projects" (Section 4b.1) |
| Cover image caching? | ❌ Prohibited — thumbnails cannot be stored locally (Section 5e.1) |
| Suitable as fallback metadata source? | ⚠️ Yes, with significant restrictions — transient enrichment only |

## Detailed Findings

### 1. Metadata Caching — Permanent Copies Prohibited

The Google APIs ToS Section 5e.1 states:

> "you will not [...] **Scrape, build databases, or otherwise create permanent
> copies of such content, or keep cached copies longer than permitted by the
> cache header**"

This applies to ALL content returned by the API, including text metadata
(title, author, page count, description). Toku's architecture stores book
metadata permanently in a local SQLite database, which directly conflicts
with the "no permanent copies" clause.

**Interpretation for Toku**: Using Google Books for one-time field
enrichment — fetch metadata, merge fields into the user's record (which
becomes user-owned data), then discard the raw API response — is the
safest approach. Once a field is saved as part of the user's library entry
with provenance tracked as `google_books`, it is arguably user-curated data
rather than a cached API response. However, this interpretation has not been
tested legally.

### 2. Free Tier Quota — 1,000 Requests/Day per API Key

| Access Type | Quota | Notes |
|---|---|---|
| With API key (free tier) | 1,000 req/day | Resets at midnight Pacific Time |
| Without API key | Undocumented, very low | Throttled or blocked; not reliable |
| Paid (billing enabled) | Up to 10,000 req/day | Requires Google Cloud billing account |

The 1,000 req/day free quota is adequate for a personal book manager.
Even heavy use (adding 10 books/day with fallback lookups) would stay well
under the limit.

**Key concern**: Unauthenticated access (no API key) is unreliable and
officially discouraged. Every user needs their own API key to use
Google Books as a fallback source.

### 3. API Key Embedding — Prohibited in Open Source

Google APIs ToS Section 4b.1:

> "**Developer credentials** (such as passwords, keys, and client IDs) are
> intended to be used by you and identify your API Client. [...] **Developer
> credentials may not be embedded in open source projects.**"

This is unambiguous. Toku cannot ship with a bundled Google Books API key.

**Mitigation**: Users must provide their own API key via configuration:

```toml
# config.toml
[metadata.google_books]
api_key = "AIza..."
```

This adds friction to the Google Books fallback feature. Users must:

1. Create a Google Cloud project
2. Enable the Books API
3. Generate an API key
4. Add it to their Toku config

### 4. Cover Image Caching — Prohibited

As documented in
[cover-image-licensing.md](cover-image-licensing.md), Section 5e.1
prohibits permanent local storage of thumbnails. Google Books cannot be
used for cover images in Toku's offline-first architecture.

### 5. Fees Clause — Cannot Charge Users

The Google Books API-specific ToS (separate from the general Google APIs
ToS) states:

> "You may not charge users any fee for the use of your application,
> unless you have entered into a separate agreement with Google or
> obtained Google's written permission."

Since Toku is free and open-source (MIT license), this is not an issue.
However, it would become relevant if Toku ever offered a paid tier that
used Google Books data.

### 6. Attribution Requirements

The Google Books branding guidelines require:

- "Powered by Google" logo adjacent to book results sourced from Google
- Prominent links to Google Books pages for each result
- No reordering or alteration of results

These requirements conflict with Toku's design principles (no external
branding, offline display of enriched data, user data ownership).

**Mitigation**: Attribution could be shown in `toku show` output when a
field's provenance is `google_books` (e.g., "Description via Google
Books"). The branding requirements apply primarily to search results
displayed in a web UI, not to individual enriched metadata fields in a CLI.

### 7. Content Removal

> "You must remove from your site or application any content provided
> through the Books API that is alleged to infringe the rights of third
> parties"

For a personal, offline-first app where the user owns their data, this is
low risk but worth noting. If Google requests content removal, the user's
local data would need a mechanism to mark and optionally remove
Google-sourced fields.

## Impact on Architecture (ADR-004)

### Recommended approach: Transient enrichment only

```text

1. Query Open Library (primary) → fetch metadata
2. If fields missing, query Google Books (fallback)
3. Merge non-empty fields into user's book record
4. Track provenance: field_name → "google_books"
5. DISCARD raw Google Books API response (do not cache)
6. User's record is now user-owned data
```

### What changes from current ADR-004

| ADR-004 statement | Update needed |
|---|---|
| "Caching: API responses cached locally for 30 days" | ❌ Cannot cache Google Books responses beyond HTTP cache headers |
| "Google Books as fallback" | ✅ Viable for text metadata, with user-provided API key |
| "Cover images: Google Books thumbnails as fallback" | ❌ Cannot cache thumbnails locally |

### Configuration design

```toml
[metadata]
# Primary source — always used, no key needed
primary = "openlibrary"

[metadata.google_books]
# Optional fallback — requires user-provided API key
enabled = false
api_key = ""  # User must supply their own
```

## Recommendation

Google Books remains viable as a **metadata-only fallback** with these
constraints:

1. **User must provide their own API key** — cannot be bundled
2. **Transient enrichment only** — fetch, merge fields, discard response
3. **No cover images** — Open Library or user-provided only
4. **Provenance tracking** — mark Google-sourced fields for potential removal
5. **Attribution** — show "via Google Books" in CLI when displaying
   Google-sourced fields

Given the friction (API key setup) and restrictions (no caching, no covers,
attribution), Google Books as a fallback should be **deferred** until Open
Library's coverage gaps cause real user pain. Open Library is sufficient as
the sole metadata source for the MVP.

## Sources Reviewed

| Source | URL |
|---|---|
| Google APIs Terms of Service | <https://developers.google.com/terms> |
| Google Books API Terms | <https://developers.google.com/books/terms> |
| Google Books API Documentation | <https://developers.google.com/books/docs/v1/using> |
| Google Books Branding Guidelines | <https://developers.google.com/books/branding> |
| Cover Image Licensing (prior validation) | [cover-image-licensing.md](cover-image-licensing.md) |

> **Disclaimer**: This document reflects a good-faith review of publicly
> available provider terms as of May 2026. It is not legal advice. Terms
> may change; verify current terms before implementing new integrations.
