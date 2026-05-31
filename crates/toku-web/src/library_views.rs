//! HTML views for the library browser, book detail, and search pages.

use maud::{DOCTYPE, Markup, PreEscaped, html};
use toku_core::{BookFormat, ContributorRole, ReadingStatus, TagCount, TagType};

use crate::library_handlers::{BookCard, BookDetailData};

// ── Shared layout ───────────────────────────────────────────────────

fn base(title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " — Toku" }
                style { (PreEscaped(CSS)) }
            }
            body {
                header.site-header {
                    div.header-inner {
                        a.logo href="/" { "📚 Toku" }
                        nav {
                            a.nav-link href="/library" { "Library" }
                            " "
                            a.nav-link href="/search" { "Search" }
                            " "
                            a.nav-link href="/stats" { "Dashboard" }
                            " "
                            a.nav-link href="/import" { "Import" }
                        }
                    }
                }
                main { (content) }
                footer.site-footer {
                    p { "Toku — your private reading tracker" }
                }
            }
        }
    }
}

// ── Rating helpers ──────────────────────────────────────────────────

fn star_rating(rating: Option<i32>) -> Markup {
    let r = match rating {
        Some(r) if r > 0 => r,
        _ => {
            return html! {
                span.rating.rating-none { "No rating" }
            };
        }
    };
    let full = r / 2;
    let half = r % 2;
    let empty = 5 - full - half;
    html! {
        span.rating title=(format!("{}.{}/5", r / 2, if r % 2 == 1 { "5" } else { "0" })) {
            @for _ in 0..full { "★" }
            @if half > 0 { "½" }
            @for _ in 0..empty { "☆" }
        }
    }
}

fn status_badge(status: &ReadingStatus) -> Markup {
    let (label, class) = match status {
        ReadingStatus::WantToRead => ("Want to Read", "badge-want"),
        ReadingStatus::Reading => ("Reading", "badge-reading"),
        ReadingStatus::Read => ("Read", "badge-read"),
        ReadingStatus::Abandoned => ("Abandoned", "badge-abandoned"),
        ReadingStatus::OnHold => ("On Hold", "badge-onhold"),
    };
    html! { span class={"badge " (class)} { (label) } }
}

fn format_icon(fmt: &BookFormat) -> &'static str {
    match fmt {
        BookFormat::Physical => "📖",
        BookFormat::Ebook => "📱",
        BookFormat::Audiobook => "🎧",
    }
}

// ── Library page ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn library_page(
    books: &[BookCard],
    view: &str,
    status: Option<&str>,
    tag: Option<&str>,
    sort: &str,
    page: usize,
    total: usize,
    total_pages: usize,
    tag_counts: &[TagCount],
) -> Markup {
    let content = html! {
        div.library-page {
            div.library-header {
                h1 { "Library" }
                span.book-count { (total) " book" @if total != 1 { "s" } }
            }

            // Filter bar
            form.filter-bar action="/library" method="get" {
                // Preserve view mode
                input type="hidden" name="view" value=(view);

                div.filter-group {
                    label for="status-filter" { "Status" }
                    select id="status-filter" name="status" onchange="this.form.submit()" {
                        option value="" selected[status.is_none()] { "All" }
                        @for (val, label) in STATUS_OPTIONS {
                            option value=(val) selected[status == Some(val)] { (label) }
                        }
                    }
                }

                div.filter-group {
                    label for="tag-filter" { "Tag" }
                    select id="tag-filter" name="tag" onchange="this.form.submit()" {
                        option value="" selected[tag.is_none()] { "All" }
                        @for tc in tag_counts {
                            option value=(&tc.name) selected[tag == Some(tc.name.as_str())] {
                                (&tc.name) " (" (tc.count) ")"
                            }
                        }
                    }
                }

                div.filter-group {
                    label for="sort-select" { "Sort" }
                    select id="sort-select" name="sort" onchange="this.form.submit()" {
                        option value="title" selected[sort == "title"] { "Title" }
                        option value="author" selected[sort == "author"] { "Author" }
                        option value="rating" selected[sort == "rating"] { "Rating" }
                        option value="added" selected[sort == "added"] { "Date Added" }
                    }
                }

                div.view-toggle {
                    a class={"view-btn" @if view == "grid" { " active" }}
                      href=(view_link("grid", status, tag, sort)) { "▦" }
                    a class={"view-btn" @if view == "list" { " active" }}
                      href=(view_link("list", status, tag, sort)) { "☰" }
                }
            }

            // Book content
            @if books.is_empty() {
                div.empty-state {
                    p { "No books found." }
                    @if status.is_some() || tag.is_some() {
                        a href="/library" { "Clear filters" }
                    }
                }
            } @else if view == "list" {
                (book_list_table(books))
            } @else {
                (book_grid(books))
            }

            // Pagination
            @if total_pages > 1 {
                (pagination(page, total_pages, status, tag, sort, view))
            }
        }
    };
    base("Library", content)
}

fn book_grid(books: &[BookCard]) -> Markup {
    html! {
        div.book-grid {
            @for card in books {
                a.book-card href=(format!("/books/{}", card.book.id)) {
                    div.card-cover {
                        @if let Some(hash) = &card.book.cover_hash {
                            img src=(format!("/covers/{hash}"))
                                alt=(format!("Cover of {}", card.book.title))
                                loading="lazy";
                        } @else {
                            div.cover-placeholder {
                                span { (format_icon(&card.book.format)) }
                            }
                        }
                    }
                    div.card-info {
                        div.card-title { (&card.book.title) }
                        @if !card.author_display.is_empty() {
                            div.card-author { (&card.author_display) }
                        }
                        (star_rating(card.book.rating))
                        (status_badge(&card.book.status))
                    }
                }
            }
        }
    }
}

fn book_list_table(books: &[BookCard]) -> Markup {
    html! {
        div.table-wrap {
            table.book-table {
                thead {
                    tr {
                        th { "Title" }
                        th { "Author" }
                        th { "Status" }
                        th { "Rating" }
                        th { "Format" }
                        th.col-pages { "Pages" }
                        th.col-date { "Added" }
                    }
                }
                tbody {
                    @for card in books {
                        tr {
                            td {
                                a href=(format!("/books/{}", card.book.id)) {
                                    (&card.book.title)
                                }
                            }
                            td { (&card.author_display) }
                            td { (status_badge(&card.book.status)) }
                            td { (star_rating(card.book.rating)) }
                            td { (format_icon(&card.book.format)) " " (card.book.format.as_str()) }
                            td.col-pages {
                                @if let Some(p) = card.book.page_count {
                                    (p)
                                }
                            }
                            td.col-date {
                                (card.book.created_at.format("%Y-%m-%d"))
                            }
                        }
                    }
                }
            }
        }
    }
}

fn pagination(
    page: usize,
    total_pages: usize,
    status: Option<&str>,
    tag: Option<&str>,
    sort: &str,
    view: &str,
) -> Markup {
    html! {
        nav.pagination {
            @if page > 1 {
                a.page-link href=(page_link(page - 1, status, tag, sort, view)) { "← Prev" }
            }
            span.page-info {
                "Page " (page) " of " (total_pages)
            }
            @if page < total_pages {
                a.page-link href=(page_link(page + 1, status, tag, sort, view)) { "Next →" }
            }
        }
    }
}

fn page_link(
    page: usize,
    status: Option<&str>,
    tag: Option<&str>,
    sort: &str,
    view: &str,
) -> String {
    let mut url = format!("/library?page={page}&view={view}&sort={sort}");
    if let Some(s) = status {
        url.push_str(&format!("&status={}", urlencoding::encode(s)));
    }
    if let Some(t) = tag {
        url.push_str(&format!("&tag={}", urlencoding::encode(t)));
    }
    url
}

fn view_link(view: &str, status: Option<&str>, tag: Option<&str>, sort: &str) -> String {
    let mut url = format!("/library?view={view}&sort={sort}");
    if let Some(s) = status {
        url.push_str(&format!("&status={}", urlencoding::encode(s)));
    }
    if let Some(t) = tag {
        url.push_str(&format!("&tag={}", urlencoding::encode(t)));
    }
    url
}

// ── Book detail page ────────────────────────────────────────────────

pub fn book_detail_page(detail: &BookDetailData) -> Markup {
    let book = &detail.book;
    let authors_display: Vec<String> = detail
        .authors
        .iter()
        .map(|(a, ba)| {
            if ba.role == ContributorRole::Author {
                a.name.clone()
            } else {
                format!("{} ({})", a.name, ba.role)
            }
        })
        .collect();

    let general_tags: Vec<&str> = detail
        .tags
        .iter()
        .filter(|t| t.tag_type == TagType::General)
        .map(|t| t.name.as_str())
        .collect();
    let mood_tags: Vec<&str> = detail
        .tags
        .iter()
        .filter(|t| t.tag_type == TagType::Mood)
        .map(|t| t.name.as_str())
        .collect();
    let pace_tags: Vec<&str> = detail
        .tags
        .iter()
        .filter(|t| t.tag_type == TagType::Pace)
        .map(|t| t.name.as_str())
        .collect();
    let cw_tags: Vec<&str> = detail
        .tags
        .iter()
        .filter(|t| t.tag_type == TagType::ContentWarning)
        .map(|t| t.name.as_str())
        .collect();

    let content = html! {
        div.detail-page {
            a.back-link href="/library" { "← Back to Library" }

            div.detail-layout {
                // Cover
                div.detail-cover {
                    @if let Some(hash) = &book.cover_hash {
                        img src=(format!("/covers/{hash}"))
                            alt=(format!("Cover of {}", book.title));
                    } @else {
                        div.cover-placeholder-lg {
                            span { (format_icon(&book.format)) }
                        }
                    }
                }

                // Metadata
                div.detail-meta {
                    h1 { (&book.title) }
                    @if let Some(sub) = &book.subtitle {
                        p.subtitle { (sub) }
                    }

                    @if !authors_display.is_empty() {
                        p.authors {
                            "by "
                            @for (i, name) in authors_display.iter().enumerate() {
                                @if i > 0 { ", " }
                                (name)
                            }
                        }
                    }

                    div.meta-row {
                        (status_badge(&book.status))
                        " "
                        (star_rating(book.rating))
                    }

                    div.meta-details {
                        @if let Some(pages) = book.page_count {
                            span.meta-item { "📄 " (pages) " pages" }
                        }
                        @if let Some(mins) = book.duration_minutes {
                            span.meta-item {
                                "🎧 " (mins / 60) "h " (mins % 60) "m"
                            }
                        }
                        span.meta-item { (format_icon(&book.format)) " " (book.format.as_str()) }
                        @if let Some(lang) = &book.language {
                            span.meta-item { "🌐 " (lang) }
                        }
                        @if let Some(date) = &book.pub_date {
                            span.meta-item { "📅 " (date) }
                        }
                    }

                    // Progress
                    @if let Some(progress) = &detail.latest_progress {
                        div.progress-section {
                            h3 { "Current Progress" }
                            p {
                                (progress.progress_type) ": " (progress.value)
                                @if let Some(pages) = book.page_count {
                                    @if progress.progress_type == toku_core::ProgressType::Page {
                                        " / " (pages)
                                        @if pages > 0 {
                                            " (" (progress.value * 100 / pages) "%)"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Description
                    @if let Some(desc) = &book.description {
                        div.description {
                            h3 { "Description" }
                            p { (desc) }
                        }
                    }

                    // Tags
                    @if !detail.tags.is_empty() {
                        div.tags-section {
                            h3 { "Tags" }
                            @if !general_tags.is_empty() {
                                div.tag-group {
                                    @for t in &general_tags {
                                        span.tag.tag-general { (t) }
                                    }
                                }
                            }
                            @if !mood_tags.is_empty() {
                                div.tag-group {
                                    span.tag-label { "Mood: " }
                                    @for t in &mood_tags {
                                        span.tag.tag-mood { (t) }
                                    }
                                }
                            }
                            @if !pace_tags.is_empty() {
                                div.tag-group {
                                    span.tag-label { "Pace: " }
                                    @for t in &pace_tags {
                                        span.tag.tag-pace { (t) }
                                    }
                                }
                            }
                            @if !cw_tags.is_empty() {
                                div.tag-group {
                                    span.tag-label { "CW: " }
                                    @for t in &cw_tags {
                                        span.tag.tag-cw { (t) }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Reading Sessions
            @if !detail.sessions.is_empty() {
                div.sessions-section {
                    h2 { "Reading Sessions" }
                    table.session-table {
                        thead {
                            tr {
                                th { "Started" }
                                th { "Finished" }
                                th { "Rating" }
                                th { "Notes" }
                            }
                        }
                        tbody {
                            @for session in &detail.sessions {
                                tr {
                                    td { (session.started_at.format("%Y-%m-%d")) }
                                    td {
                                        @if let Some(f) = session.finished_at {
                                            (f.format("%Y-%m-%d"))
                                        } @else {
                                            em { "In progress" }
                                        }
                                    }
                                    td { (star_rating(session.rating)) }
                                    td.notes-cell {
                                        @if let Some(notes) = &session.notes {
                                            (notes)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Reading Progress Log
            @if !detail.progress_log.is_empty() {
                div.progress-log-section {
                    h2 { "Progress Log" }
                    table.progress-table {
                        thead {
                            tr {
                                th { "Date" }
                                th { "Type" }
                                th { "Value" }
                                th { "Note" }
                            }
                        }
                        tbody {
                            @for entry in &detail.progress_log {
                                tr {
                                    td { (entry.logged_at.format("%Y-%m-%d %H:%M")) }
                                    td { (entry.progress_type) }
                                    td { (entry.value) }
                                    td.notes-cell {
                                        @if let Some(n) = &entry.note {
                                            (n)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    base(&book.title, content)
}

// ── Search page ─────────────────────────────────────────────────────

pub fn search_page(
    q: Option<&str>,
    status: Option<&str>,
    tag: Option<&str>,
    results: &[BookCard],
    tag_counts: &[TagCount],
) -> Markup {
    let content = html! {
        div.search-page {
            h1 { "Search" }

            form.search-form action="/search" method="get" {
                div.search-input-row {
                    input type="search" name="q" placeholder="Search books…"
                          value=(q.unwrap_or("")) autofocus;
                    button type="submit" { "Search" }
                }
                div.search-filters {
                    div.filter-group {
                        label for="search-status" { "Status" }
                        select id="search-status" name="status" {
                            option value="" selected[status.is_none()] { "All" }
                            @for (val, label) in STATUS_OPTIONS {
                                option value=(val) selected[status == Some(val)] { (label) }
                            }
                        }
                    }
                    div.filter-group {
                        label for="search-tag" { "Tag" }
                        select id="search-tag" name="tag" {
                            option value="" selected[tag.is_none()] { "All" }
                            @for tc in tag_counts {
                                option value=(&tc.name) selected[tag == Some(tc.name.as_str())] {
                                    (&tc.name) " (" (tc.count) ")"
                                }
                            }
                        }
                    }
                }
            }

            @if let Some(query) = q {
                @if !query.is_empty() {
                    p.search-summary {
                        (results.len()) " result" @if results.len() != 1 { "s" }
                        " for "" (query) """
                    }

                    @if results.is_empty() {
                        div.empty-state {
                            p { "No books matched your search." }
                        }
                    } @else {
                        div.search-results {
                            @for card in results {
                                a.search-result href=(format!("/books/{}", card.book.id)) {
                                    div.result-cover {
                                        @if let Some(hash) = &card.book.cover_hash {
                                            img src=(format!("/covers/{hash}"))
                                                alt="" loading="lazy";
                                        } @else {
                                            div.cover-placeholder-sm {
                                                (format_icon(&card.book.format))
                                            }
                                        }
                                    }
                                    div.result-info {
                                        div.result-title { (&card.book.title) }
                                        @if !card.author_display.is_empty() {
                                            div.result-author { (&card.author_display) }
                                        }
                                        div.result-meta {
                                            (status_badge(&card.book.status))
                                            " "
                                            (star_rating(card.book.rating))
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    base("Search", content)
}

// ── Constants ───────────────────────────────────────────────────────

const STATUS_OPTIONS: &[(&str, &str)] = &[
    ("want-to-read", "Want to Read"),
    ("reading", "Reading"),
    ("read", "Read"),
    ("abandoned", "Abandoned"),
    ("on-hold", "On Hold"),
];

// ── CSS ─────────────────────────────────────────────────────────────

const CSS: &str = r#"
:root {
    --bg: #fafafa;
    --bg-card: #ffffff;
    --text: #1a1a2e;
    --text-secondary: #64748b;
    --border: #e2e8f0;
    --accent: #6366f1;
    --accent-hover: #4f46e5;
    --badge-want: #3b82f6;
    --badge-reading: #10b981;
    --badge-read: #8b5cf6;
    --badge-abandoned: #ef4444;
    --badge-onhold: #f59e0b;
    --tag-bg: #eef2ff;
    --tag-text: #4338ca;
    --tag-mood-bg: #fef3c7;
    --tag-mood-text: #92400e;
    --tag-pace-bg: #d1fae5;
    --tag-pace-text: #065f46;
    --tag-cw-bg: #fee2e2;
    --tag-cw-text: #991b1b;
}

@media (prefers-color-scheme: dark) {
    :root {
        --bg: #0f172a;
        --bg-card: #1e293b;
        --text: #e2e8f0;
        --text-secondary: #94a3b8;
        --border: #334155;
        --accent: #818cf8;
        --accent-hover: #6366f1;
        --badge-want: #60a5fa;
        --badge-reading: #34d399;
        --badge-read: #a78bfa;
        --badge-abandoned: #f87171;
        --badge-onhold: #fbbf24;
        --tag-bg: #312e81;
        --tag-text: #c7d2fe;
        --tag-mood-bg: #78350f;
        --tag-mood-text: #fde68a;
        --tag-pace-bg: #064e3b;
        --tag-pace-text: #a7f3d0;
        --tag-cw-bg: #7f1d1d;
        --tag-cw-text: #fecaca;
    }
}

* { margin: 0; padding: 0; box-sizing: border-box; }

body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
                 "Helvetica Neue", Arial, sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.6;
}

a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }

/* Header */
.site-header {
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
    padding: 0.75rem 1.5rem;
}
.header-inner {
    max-width: 1200px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
}
.logo { font-size: 1.25rem; font-weight: 700; color: var(--text); text-decoration: none; }
.nav-link { color: var(--accent); text-decoration: none; font-weight: 500; }
.nav-link:hover { text-decoration: underline; }

/* Footer */
.site-footer { text-align: center; padding: 2rem 1rem; color: var(--text-secondary); font-size: 0.85rem; }

/* Main */
main { max-width: 1200px; margin: 0 auto; padding: 1.5rem; }

/* ── Library page ─────────────────────────────── */

.library-header {
    display: flex; align-items: baseline; gap: 1rem;
    margin-bottom: 1rem;
}
.library-header h1 { font-size: 1.75rem; }
.book-count { color: var(--text-secondary); font-size: 0.9rem; }

.filter-bar {
    display: flex; align-items: flex-end; gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 1.5rem;
    padding: 0.75rem 1rem;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
}
.filter-group { display: flex; flex-direction: column; gap: 0.25rem; }
.filter-group label { font-size: 0.75rem; color: var(--text-secondary); font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; }
.filter-group select {
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font-size: 0.9rem;
}

.view-toggle {
    display: flex; gap: 0.25rem; margin-left: auto; align-self: flex-end;
}
.view-btn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 36px; height: 36px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text-secondary);
    text-decoration: none;
    font-size: 1.1rem;
}
.view-btn.active, .view-btn:hover {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
}

/* Grid view */
.book-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 1.25rem;
}

.book-card {
    display: flex; flex-direction: column;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    text-decoration: none;
    color: var(--text);
    transition: transform 0.15s, box-shadow 0.15s;
}
.book-card:hover {
    transform: translateY(-3px);
    box-shadow: 0 8px 25px rgba(0,0,0,0.1);
    text-decoration: none;
}

.card-cover {
    aspect-ratio: 2 / 3;
    overflow: hidden;
    background: var(--border);
}
.card-cover img {
    width: 100%; height: 100%;
    object-fit: cover;
}

.cover-placeholder, .cover-placeholder-lg, .cover-placeholder-sm {
    display: flex; align-items: center; justify-content: center;
    background: var(--border);
    color: var(--text-secondary);
}
.cover-placeholder { width: 100%; height: 100%; font-size: 2.5rem; }
.cover-placeholder-lg { width: 100%; aspect-ratio: 2/3; font-size: 4rem; border-radius: 8px; }
.cover-placeholder-sm { width: 60px; height: 90px; font-size: 1.5rem; border-radius: 4px; flex-shrink: 0; }

.card-info { padding: 0.6rem 0.75rem; }
.card-title { font-weight: 600; font-size: 0.85rem; margin-bottom: 0.15rem; overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; }
.card-author { font-size: 0.78rem; color: var(--text-secondary); margin-bottom: 0.3rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.rating { color: #f59e0b; font-size: 0.8rem; }
.rating-none { color: var(--text-secondary); font-size: 0.75rem; font-style: italic; }

/* Badges */
.badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 12px;
    font-size: 0.7rem;
    font-weight: 600;
    color: white;
}
.badge-want { background: var(--badge-want); }
.badge-reading { background: var(--badge-reading); }
.badge-read { background: var(--badge-read); }
.badge-abandoned { background: var(--badge-abandoned); }
.badge-onhold { background: var(--badge-onhold); }

/* List view */
.table-wrap { overflow-x: auto; }
.book-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9rem;
}
.book-table th, .book-table td {
    padding: 0.6rem 0.75rem;
    text-align: left;
    border-bottom: 1px solid var(--border);
}
.book-table th {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
    font-weight: 600;
    background: var(--bg-card);
    position: sticky;
    top: 0;
}
.book-table tbody tr:hover { background: var(--bg-card); }
.col-pages, .col-date { white-space: nowrap; }

/* Pagination */
.pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    margin-top: 2rem;
    padding: 1rem;
}
.page-link {
    padding: 0.4rem 1rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-card);
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
}
.page-link:hover { background: var(--accent); color: white; border-color: var(--accent); text-decoration: none; }
.page-info { color: var(--text-secondary); font-size: 0.9rem; }

/* Empty state */
.empty-state { text-align: center; padding: 3rem 1rem; color: var(--text-secondary); }

/* ── Book detail ──────────────────────────────── */

.detail-page { max-width: 960px; margin: 0 auto; }

.back-link {
    display: inline-block;
    margin-bottom: 1.5rem;
    color: var(--accent);
    font-size: 0.9rem;
}

.detail-layout {
    display: grid;
    grid-template-columns: 240px 1fr;
    gap: 2rem;
    margin-bottom: 2rem;
}

.detail-cover img {
    width: 100%;
    border-radius: 8px;
    box-shadow: 0 4px 15px rgba(0,0,0,0.15);
}

.detail-meta h1 { font-size: 1.6rem; margin-bottom: 0.25rem; }
.detail-meta .subtitle { color: var(--text-secondary); font-size: 1.1rem; margin-bottom: 0.5rem; }
.detail-meta .authors { font-size: 1rem; color: var(--text-secondary); margin-bottom: 0.75rem; }
.meta-row { display: flex; align-items: center; gap: 0.75rem; margin-bottom: 1rem; }

.meta-details {
    display: flex; flex-wrap: wrap; gap: 0.75rem;
    margin-bottom: 1rem;
    font-size: 0.9rem;
}
.meta-item {
    background: var(--bg-card);
    border: 1px solid var(--border);
    padding: 0.3rem 0.7rem;
    border-radius: 6px;
}

.description { margin-bottom: 1.5rem; }
.description h3, .tags-section h3, .progress-section h3 { font-size: 1rem; margin-bottom: 0.5rem; }
.description p { color: var(--text-secondary); line-height: 1.7; }

.progress-section {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1rem;
    margin-bottom: 1rem;
}

/* Tags */
.tags-section { margin-bottom: 1.5rem; }
.tag-group { display: flex; flex-wrap: wrap; gap: 0.4rem; align-items: center; margin-bottom: 0.5rem; }
.tag-label { font-size: 0.8rem; color: var(--text-secondary); font-weight: 600; margin-right: 0.25rem; }
.tag {
    display: inline-block;
    padding: 0.2rem 0.6rem;
    border-radius: 12px;
    font-size: 0.78rem;
    font-weight: 500;
}
.tag-general { background: var(--tag-bg); color: var(--tag-text); }
.tag-mood { background: var(--tag-mood-bg); color: var(--tag-mood-text); }
.tag-pace { background: var(--tag-pace-bg); color: var(--tag-pace-text); }
.tag-cw { background: var(--tag-cw-bg); color: var(--tag-cw-text); }

/* Sessions & progress tables */
.sessions-section, .progress-log-section { margin-bottom: 2rem; }
.sessions-section h2, .progress-log-section h2 { font-size: 1.25rem; margin-bottom: 0.75rem; }

.session-table, .progress-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9rem;
}
.session-table th, .session-table td,
.progress-table th, .progress-table td {
    padding: 0.5rem 0.75rem;
    text-align: left;
    border-bottom: 1px solid var(--border);
}
.session-table th, .progress-table th {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
    font-weight: 600;
}
.notes-cell { max-width: 300px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* ── Search ───────────────────────────────────── */

.search-page h1 { font-size: 1.75rem; margin-bottom: 1rem; }

.search-form { margin-bottom: 1.5rem; }
.search-input-row {
    display: flex; gap: 0.5rem; margin-bottom: 0.75rem;
}
.search-input-row input {
    flex: 1;
    padding: 0.6rem 1rem;
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 1rem;
    background: var(--bg-card);
    color: var(--text);
}
.search-input-row input:focus { outline: 2px solid var(--accent); border-color: var(--accent); }
.search-input-row button {
    padding: 0.6rem 1.5rem;
    background: var(--accent);
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
}
.search-input-row button:hover { background: var(--accent-hover); }

.search-filters { display: flex; gap: 1rem; flex-wrap: wrap; }

.search-summary { color: var(--text-secondary); margin-bottom: 1rem; font-size: 0.95rem; }

.search-results { display: flex; flex-direction: column; gap: 0.75rem; }
.search-result {
    display: flex; gap: 1rem; align-items: flex-start;
    padding: 0.75rem 1rem;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    text-decoration: none;
    color: var(--text);
    transition: border-color 0.15s;
}
.search-result:hover { border-color: var(--accent); text-decoration: none; }

.result-cover img { width: 60px; height: 90px; object-fit: cover; border-radius: 4px; }
.result-title { font-weight: 600; margin-bottom: 0.15rem; }
.result-author { font-size: 0.85rem; color: var(--text-secondary); margin-bottom: 0.3rem; }
.result-meta { display: flex; gap: 0.5rem; align-items: center; }

/* ── Responsive ───────────────────────────────── */

@media (max-width: 768px) {
    .detail-layout { grid-template-columns: 1fr; }
    .detail-cover { max-width: 200px; }
    .book-grid { grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 0.75rem; }
    .filter-bar { flex-direction: column; align-items: stretch; }
    .view-toggle { margin-left: 0; justify-content: flex-end; }
    .header-inner { flex-direction: column; gap: 0.5rem; }
    .col-pages, .col-date { display: none; }
}
"#;
