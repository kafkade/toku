# 📚 Toku

**A private, offline-first personal book manager.**

Track what you read. Own your data. No accounts. No social features. No cloud dependency.

Toku combines the metadata depth of [Calibre](https://calibre-ebook.com/), the reading
tracking of [Goodreads](https://www.goodreads.com/), and the analytics of
[StoryGraph](https://www.thestorygraph.com/) — in a single CLI tool that keeps everything
on your machine.

```sh
toku add --isbn 9780441013593          # Add a book by ISBN
toku add --title "Dune" -T sci-fi      # Add with tags
toku import goodreads ~/export.csv     # Import your Goodreads library
toku reading start "Dune" --page 1     # Start tracking progress
toku reading update "Dune" --page 145  # Log where you are
toku reading finish "Dune" --rating 5  # Done — rate it
toku stats --year 2025                 # See your reading stats
toku browse                            # Interactive TUI browser
```

> **Status**: Active development (v0.2.1) — full-featured CLI, web dashboard,
> macOS/iOS apps, and Windows desktop app. Phases 0–5 complete.

---

## Why "Toku"?

**Toku** (読く) comes from the Japanese kanji **読** — meaning _to read_.

The name is also a nod to **積ん読 (tsundoku)** — a Japanese word with no English
equivalent that describes the act of acquiring books and letting them pile up unread.
It's not a criticism; it's a shared, self-aware feeling every reader knows. You buy
faster than you read. The stack grows. You love every unread spine on the shelf anyway.

Toku is the tool for the reader who wants to see the full picture: what they've read,
what they're reading, and — yes — everything they've been meaning to get to. It tracks
your progress without judgment, keeps your data private, and never asks you to share,
follow, or perform your reading for an audience.

The name captures both the act of reading and the honest relationship readers have with
their ever-growing libraries.

---

## Principles

- **Your data, your machine.** Everything lives in a local SQLite database. No accounts,
  no servers, no cloud required. Back it up however you like.
- **No social features.** No friends, followers, feeds, or book clubs. Reading is private.
  Your library is for you.
- **Import everything.** Bring your Goodreads, Calibre, or StoryGraph history. Years of
  reading data should transfer in seconds.
- **CLI-first.** A fast, scriptable command-line tool. Web, iOS, macOS, and Windows
  interfaces are planned for later — built on the same core library.
- **Open source.** MIT licensed. Contributions welcome.

## Features

- 📖 Add books manually, by ISBN, or by Open Library search
- 📊 Reading progress tracking (pages, percentage, chapters, audiobook time)
- 🏷️ Tags for organizing your library (imported Goodreads shelves become tags)
- 📈 Reading statistics and analytics (pace, format breakdown, yearly filtering)
- 📥 Import from Goodreads CSV, Calibre, and StoryGraph (with dry-run, dedup, and tag preservation)
- 📤 Export to CSV, JSON, Markdown, and canonical backup (ZIP)
- 🔍 Full-text search across your entire library (titles, authors, descriptions)
- 🖥️ Interactive TUI browser with split-pane layout, filters, and live detail view
- 🔄 Bulk operations for tagging, status changes, and deletions across filtered sets
- 🌐 Web dashboard (`toku serve`) with statistics, import wizard, library views, and dark mode
- 🍎 macOS app (SwiftUI) with sidebar navigation, sortable table/grid views, and Swift Charts
- 📱 iOS app (iPhone + iPad) with cover grid, barcode scanner, and reading progress updates
- 🪟 Windows desktop app (Tauri v2) wrapping the web UI in a native window with system tray
- 🔄 Optional multi-device sync via a self-hostable relay server
  ([deployment guide](docs/sync-server.md)) with end-to-end encryption

## Tech Stack

- **Language**: Rust
- **Database**: SQLite with FTS5
- **CLI**: clap v4
- **Web**: Axum + maud (server-side rendered HTML with inline SVG charts)
- **Desktop**: Tauri v2 (Windows)
- **Mobile/macOS**: SwiftUI via Rust FFI (`toku-ffi` crate)
- **Architecture**: Cargo workspace with 9 crates — core, database, import, export,
  metadata, CLI, FFI, web, and desktop

## License

[MIT](LICENSE)

## Acknowledgments

- Book metadata and cover images from [Open Library](https://openlibrary.org) by the
  [Internet Archive](https://archive.org).

---

_Built by [kafkade](https://github.com/kafkade)._
