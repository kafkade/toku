## Description

<!-- What does this PR do? Provide a brief summary of the changes. -->

## Related Issues

<!-- Link related issues: "Closes #123" or "Relates to #456" -->

## Type of Change

- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Documentation update
- [ ] Refactoring (no functional changes)
- [ ] CI / infrastructure
- [ ] Other (describe below)

## Crate

<!-- Which crate(s) does this touch? -->

- [ ] `toku-core` — Domain models, traits, state machine
- [ ] `toku-db` — SQLite persistence, migrations, FTS5
- [ ] `toku-import` — Importers (Goodreads, Calibre, StoryGraph)
- [ ] `toku-meta` — Metadata fetching (Open Library, Google Books)
- [ ] `toku-cli` — CLI binary
- [ ] `toku-export` — Exporters (CSV, JSON, Markdown, BibTeX)
- [ ] `docs/` — Documentation

## Data Integrity Checklist

<!-- Toku is a local-first tool — user data ownership is non-negotiable -->

- [ ] No user data is sent to any server without explicit opt-in
- [ ] Import operations are idempotent (re-importing the same file creates no duplicates)
- [ ] User edits to metadata are never overwritten by auto-enrichment
- [ ] New fields track provenance (source + timestamp)
- [ ] Export round-trip is preserved (if applicable)

## Checklist

- [ ] I have read [CONTRIBUTING.md](CONTRIBUTING.md)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] I have updated documentation (if applicable)
