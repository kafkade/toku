# ADR-007: Web Framework — Axum + Maud (Server-Side Rendering)

**Status**: Accepted
**Date**: 2026-05-30
**Decision**: Use Axum for the HTTP server and maud for server-side HTML rendering,
with inline SVG charts and minimal inline JavaScript. No HTMX. No Leptos.

## Context

Phase 4 requires a web interface for statistics, import, library browsing, and search.
The ROADMAP identified two candidate stacks — both keep the codebase in Rust:

- **Option A: Axum + HTMX** — Axum serves server-rendered HTML; HTMX handles dynamic
  interactions via HTML attributes with minimal JavaScript.
- **Option B: Leptos** — Full-stack Rust framework with SSR + client-side hydration
  via WASM. No handwritten JavaScript, but requires WASM output and client-side
  glue code.

A third option emerged during implementation: **Axum + maud** — server-side rendering
with a Rust macro-based HTML DSL, no template files, and no client-side framework.

## Decision

**Axum + maud** with server-side rendering, inline SVG charts, and targeted inline
JavaScript only where strictly necessary (SSE event handling for import progress).

### Stack

| Layer | Choice | Role |
|-------|--------|------|
| HTTP server | Axum 0.8 | Routing, middleware, SSE, multipart uploads |
| HTML rendering | maud 0.26 | Type-checked HTML via Rust macros |
| Charts | Inline SVG | Server-generated, no charting library |
| Interactivity | Minimal inline JS (~15 lines) | SSE `EventSource` for import progress only |
| Styling | Embedded `<style>` | CSS custom properties, `prefers-color-scheme` dark mode |
| Assets | None external | No CDN, no npm, no bundler — fully offline |

### Architecture

- `toku-web` is a **library crate**, not a binary. The CLI calls `toku_web::serve()`
  via `toku serve`.
- All HTML is rendered server-side. No hydration, no virtual DOM, no client state.
- Database access uses `tokio::task::spawn_blocking` wrapping synchronous rusqlite
  calls. Migrations run once at startup; per-request opens use
  `Database::open_no_migrate()`.
- Import progress uses Server-Sent Events (SSE) with `tokio::sync::broadcast`
  channels. The client-side EventSource listener is the only JavaScript in the app.

## Rationale

### Why not HTMX?

HTMX was the ROADMAP's default recommendation alongside Axum. We chose not to use it
because:

1. **No dynamic interactions needed yet.** The Phase 4 deliverables (statistics
   dashboard, import wizard) are read-heavy pages with full-page navigation. HTMX
   shines for partial-page updates (inline editing, live search, dynamic forms) — none
   of which are needed in the current scope.
2. **SSE is simpler than HTMX for progress streaming.** The import wizard's live
   progress bar uses native `EventSource` (4 lines of JS). HTMX's SSE extension adds
   abstraction without reducing complexity.
3. **No external web assets.** HTMX is small (~14KB) and could be vendored for
   offline use, but omitting it removes even that from the maintenance surface.
   The entire web interface ships with zero external assets — no vendored JS, no
   CDN, no npm, no bundler.
4. **Easy to adopt later.** HTMX is additive. If Phase 4+ features need partial-page
   updates (library grid filtering, inline book editing), HTMX can be added to
   specific pages without rearchitecting.

### Why not Leptos?

1. **Build complexity.** Leptos requires WASM compilation (`wasm-pack` or `trunk`),
   a separate client build step, and hydration logic. This adds CI time and a new
   failure mode for a feature set that doesn't need client-side Rust.
2. **Ecosystem maturity.** Leptos is pre-1.0 (0.7.x as of writing). API churn and
   migration burden are a risk for a solo-developer project.
3. **Contributor accessibility.** Server-rendered HTML + CSS is universally understood.
   Leptos's reactive signal system (`create_signal`, `create_effect`) has a learning
   curve even for experienced Rust developers.
4. **Offline-first already satisfied.** Toku's offline-first design means the
   SQLite database lives on the local machine. The Axum server reads it directly
   on `localhost`. A hydrated WASM client would still need to fetch data from the
   Axum backend — the WASM layer adds complexity without increasing offline
   capability beyond what a service worker provides.

### Why maud over other template engines?

1. **Compile-checked HTML construction.** Syntax errors in HTML are caught by
   `rustc` at compile time. Maud also escapes values by default, preventing
   accidental injection. (Note: it does not guarantee semantic HTML correctness
   or accessibility — those require manual review.)
2. **Raw SVG embedding.** `maud::PreEscaped` cleanly embeds server-generated SVG
   chart markup. Other template engines (askama, tera) require escaping workarounds
   for raw HTML injection.
3. **No template files.** Views are Rust functions returning `Markup`. No separate
   `.html` files to keep in sync, no template discovery at runtime.
4. **Familiar to Rust developers.** The maud macro syntax reads like a Rust DSL —
   natural for contributors already working in the crate.

### Alignment with constraints

| Constraint | How this stack satisfies it |
|------------|---------------------------|
| Local-first / offline | No CDN, no external assets, no network calls. Works on `localhost` with no internet. |
| No social features | Server renders pages for the local user only. No auth, no multi-user, no sharing. |
| User data ownership | All data stays in the local SQLite database. The web server is `127.0.0.1` by default. |
| CLI-first | `toku serve` is a CLI command. The web interface reuses the same `toku-core` and `toku-db` crates. |

## Consequences

- **No client-side routing.** Every page is a full server render. This is acceptable
  for the current feature set but may feel slow for data-dense pages (library grid
  with hundreds of books). Mitigation: add HTMX for specific interactions if needed.
- **maud's Rust 2021 prefix conflict.** The `element#id` shorthand syntax conflicts
  with Rust 2021 reserved prefixes. Use `element id="value"` instead. This is a minor
  ergonomic cost.
- **maud's axum feature targets axum-core 0.4 (axum 0.7)**, not axum-core 0.5
  (axum 0.8). Workaround: render to `Html<String>` instead of returning `Markup`
  directly. This is invisible to handler code.
- **Chart rendering is manual.** Inline SVG charts are hand-built string builders.
  If chart complexity grows significantly, consider a Rust SVG library (e.g.,
  `plotters` with SVG backend). Current scope (5 chart types) is manageable.
- **`PreEscaped` requires manual escaping.** SVG charts use `maud::PreEscaped` to
  embed raw markup. Any user-derived content (book titles, author names, tags)
  must be HTML-escaped before entering `PreEscaped` to prevent injection. The
  `charts.rs` module uses a dedicated `esc()` function for this — all new chart
  code must follow the same pattern.
- **Inline CSS/JS limits future CSP hardening.** Embedded `<style>` and
  `<script>` blocks are acceptable for Phase 4 but would conflict with a strict
  Content Security Policy. If the UI grows or is wrapped in Tauri (Phase 5),
  assets may need to move to local static files with nonces or hashes.
- **HTMX adoption path remains open.** The server-rendered architecture is fully
  compatible with adding HTMX incrementally. No rearchitecting needed.
