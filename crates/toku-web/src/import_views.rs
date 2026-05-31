//! HTML views for the import wizard.

use maud::{DOCTYPE, Markup, PreEscaped, html};
use toku_import::ImportReport;

use crate::import_handlers::ImportSourceKind;

/// Wrap content in the base HTML layout (shared with stats views).
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
                            a.nav-link href="/stats" { "Dashboard" }
                            " "
                            a.nav-link href="/import" { "Import" }
                        }
                    }
                }
                main { (content) }
            }
        }
    }
}

/// Import type selection page.
pub fn import_page() -> Markup {
    base(
        "Import Library",
        html! {
            div.import-page {
                h1 { "Import Your Library" }
                p.subtitle { "Choose an import source to get started." }

                div.import-cards {
                    // Goodreads
                    div.import-card {
                        h2 { "📗 Goodreads" }
                        p { "Import from a Goodreads CSV export." }
                        form action="/import/upload" method="post" enctype="multipart/form-data" {
                            input type="hidden" name="source" value="goodreads";
                            div.form-field {
                                label for="gr-file" { "CSV file" }
                                input id="gr-file" type="file" name="file" accept=".csv" required;
                            }
                            button.btn type="submit" { "Upload & Preview" }
                        }
                    }

                    // StoryGraph
                    div.import-card {
                        h2 { "📘 StoryGraph" }
                        p { "Import from a StoryGraph CSV export." }
                        form action="/import/upload" method="post" enctype="multipart/form-data" {
                            input type="hidden" name="source" value="storygraph";
                            div.form-field {
                                label for="sg-file" { "CSV file" }
                                input id="sg-file" type="file" name="file" accept=".csv" required;
                            }
                            button.btn type="submit" { "Upload & Preview" }
                        }
                    }

                    // Calibre
                    div.import-card {
                        h2 { "📙 Calibre" }
                        p { "Import from a Calibre library directory." }
                        form action="/import/calibre" method="post" {
                            div.form-field {
                                label for="cal-path" { "Library path" }
                                input id="cal-path" type="text" name="path"
                                      placeholder="/path/to/Calibre Library" required;
                            }
                            div.form-field.checkbox-field {
                                input id="cal-covers" type="checkbox" name="import_covers" checked;
                                label for="cal-covers" { "Import cover images" }
                            }
                            button.btn type="submit" { "Preview Import" }
                        }
                    }
                }
            }
        },
    )
}

/// Dry-run preview page.
pub fn preview_page(session_id: &str, source: &ImportSourceKind, report: &ImportReport) -> Markup {
    let source_name = match source {
        ImportSourceKind::Goodreads => "Goodreads",
        ImportSourceKind::Calibre { .. } => "Calibre",
        ImportSourceKind::StoryGraph => "StoryGraph",
    };

    base(
        &format!("{source_name} Import Preview"),
        html! {
            div.preview-page {
                h1 { (source_name) " Import Preview" }
                p.subtitle { "This is a dry run — no changes have been made." }

                // Summary counts
                section.preview-summary {
                    div.preview-stat.stat-import {
                        span.stat-value { (report.imported) }
                        span.stat-label { "to import" }
                    }
                    div.preview-stat.stat-skip {
                        span.stat-value { (report.skipped) }
                        span.stat-label { "duplicates (skip)" }
                    }
                    div.preview-stat.stat-update {
                        span.stat-value { (report.updated) }
                        span.stat-label { "to update" }
                    }
                    @if report.errors > 0 {
                        div.preview-stat.stat-error {
                            span.stat-value { (report.errors) }
                            span.stat-label { "errors" }
                        }
                    }
                }

                // Status breakdown
                @if !report.status_counts.is_empty() {
                    section.preview-section {
                        h3 { "Status Breakdown" }
                        div.status-pills {
                            @for (status, count) in &report.status_counts {
                                span.status-pill { (status) ": " (count) }
                            }
                        }
                    }
                }

                // Sample tables
                @if !report.imported_samples.is_empty() {
                    section.preview-section {
                        h3 { "Books to Import (sample)" }
                        table.sample-table {
                            thead { tr { th { "Title" } th { "Author" } th { "Status" } } }
                            tbody {
                                @for row in &report.imported_samples {
                                    tr {
                                        td { (row.title) }
                                        td { (row.author) }
                                        td { (row.status) }
                                    }
                                }
                            }
                        }
                    }
                }

                @if !report.skipped_samples.is_empty() {
                    section.preview-section {
                        h3 { "Duplicates to Skip (sample)" }
                        table.sample-table {
                            thead { tr { th { "Title" } th { "Author" } th { "Status" } } }
                            tbody {
                                @for row in &report.skipped_samples {
                                    tr.row-skipped {
                                        td { (row.title) }
                                        td { (row.author) }
                                        td { (row.status) }
                                    }
                                }
                            }
                        }
                    }
                }

                @if !report.updated_samples.is_empty() {
                    section.preview-section {
                        h3 { "Books to Update (sample)" }
                        table.sample-table {
                            thead { tr { th { "Title" } th { "Author" } th { "Status" } } }
                            tbody {
                                @for row in &report.updated_samples {
                                    tr.row-updated {
                                        td { (row.title) }
                                        td { (row.author) }
                                        td { (row.status) }
                                    }
                                }
                            }
                        }
                    }
                }

                // Warnings
                @if !report.warnings.is_empty() {
                    section.preview-section {
                        h3 { "Warnings" }
                        ul.warning-list {
                            @for w in &report.warnings {
                                li { (w) }
                            }
                        }
                    }
                }

                // Errors
                @if !report.error_details.is_empty() {
                    section.preview-section {
                        h3 { "Errors" }
                        ul.error-list {
                            @for e in report.error_details.iter().take(10) {
                                li { (e) }
                            }
                        }
                    }
                }

                // Actions
                div.preview-actions {
                    form action={ "/import/execute/" (session_id) } method="post" {
                        button.btn.btn-primary type="submit" { "Confirm Import" }
                    }
                    a.btn.btn-secondary href="/import" { "Cancel" }
                }
            }
        },
    )
}

/// Progress page with SSE-powered live updates.
pub fn progress_page(session_id: &str, source: &ImportSourceKind) -> Markup {
    let source_name = match source {
        ImportSourceKind::Goodreads => "Goodreads",
        ImportSourceKind::Calibre { .. } => "Calibre",
        ImportSourceKind::StoryGraph => "StoryGraph",
    };

    base(
        &format!("{source_name} Import"),
        html! {
            div.progress-page {
                h1 { "Importing from " (source_name) "…" }

                div.progress-container {
                    div.progress-bar-outer {
                        div id="progress-bar" class="progress-bar-inner" style="width:0%" {}
                    }
                    p id="progress-text" class="progress-text" { "Starting…" }
                }

                div.progress-counters {
                    div.counter { span id="imported-count" class="counter-value" { "0" } span.counter-label { "Imported" } }
                    div.counter { span id="skipped-count" class="counter-value" { "0" } span.counter-label { "Skipped" } }
                    div.counter { span id="updated-count" class="counter-value" { "0" } span.counter-label { "Updated" } }
                    div.counter { span id="error-count" class="counter-value" { "0" } span.counter-label { "Errors" } }
                }

                p id="current-title" class="current-title" {}
            }

            // Inline JS for SSE (minimal, offline-compatible)
            script {
                (PreEscaped(format!(r#"
(function() {{
    var id = '{session_id}';
    var src = new EventSource('/import/progress/' + id);
    src.onmessage = function(e) {{
        var d = JSON.parse(e.data);
        if (d.event_type === 'complete' || d.event_type === 'error') {{
            src.close();
            window.location.href = '/import/results/' + id;
            return;
        }}
        if (d.total > 0) {{
            var pct = Math.round((d.row / d.total) * 100);
            document.getElementById('progress-bar').style.width = pct + '%';
            document.getElementById('progress-text').textContent = d.row + ' / ' + d.total;
        }}
        document.getElementById('imported-count').textContent = d.imported;
        document.getElementById('skipped-count').textContent = d.skipped;
        document.getElementById('updated-count').textContent = d.updated;
        document.getElementById('error-count').textContent = d.errors;
        if (d.title) document.getElementById('current-title').textContent = d.title;
    }};
    src.onerror = function() {{
        src.close();
        window.location.href = '/import/results/' + id;
    }};
}})();
"#)))
            }
        },
    )
}

/// Final results page.
pub fn results_page(source: &ImportSourceKind, report: &ImportReport) -> Markup {
    let source_name = match source {
        ImportSourceKind::Goodreads => "Goodreads",
        ImportSourceKind::Calibre { .. } => "Calibre",
        ImportSourceKind::StoryGraph => "StoryGraph",
    };

    base(
        &format!("{source_name} Import Results"),
        html! {
            div.results-page {
                h1 { "Import Complete" }

                // Summary
                section.results-summary {
                    div.result-stat {
                        span.stat-value { (report.total_rows) }
                        span.stat-label { "Total Rows" }
                    }
                    div.result-stat.stat-import {
                        span.stat-value { (report.imported) }
                        span.stat-label { "Imported" }
                    }
                    div.result-stat.stat-skip {
                        span.stat-value { (report.skipped) }
                        span.stat-label { "Skipped" }
                    }
                    div.result-stat.stat-update {
                        span.stat-value { (report.updated) }
                        span.stat-label { "Updated" }
                    }
                    @if report.errors > 0 {
                        div.result-stat.stat-error {
                            span.stat-value { (report.errors) }
                            span.stat-label { "Errors" }
                        }
                    }
                }

                // Import ID
                @if let Some(ref id) = report.import_id {
                    section.import-id-section {
                        p {
                            "Import ID: " code { (id) }
                        }
                        p.undo-hint {
                            "To undo: " code { "toku import undo " (id) }
                        }
                    }
                }

                // Status breakdown
                @if !report.status_counts.is_empty() {
                    section.results-section {
                        h3 { "By Status" }
                        div.status-pills {
                            @for (status, count) in &report.status_counts {
                                span.status-pill { (status) ": " (count) }
                            }
                        }
                    }
                }

                // Samples
                @if !report.imported_samples.is_empty() {
                    section.results-section {
                        h3 { "Imported (sample)" }
                        table.sample-table {
                            thead { tr { th { "Title" } th { "Author" } th { "Status" } } }
                            tbody {
                                @for row in &report.imported_samples {
                                    tr { td { (row.title) } td { (row.author) } td { (row.status) } }
                                }
                            }
                        }
                    }
                }

                // Warnings
                @if !report.warnings.is_empty() {
                    section.results-section {
                        h3 { "Warnings (" (report.warnings.len()) ")" }
                        ul.warning-list {
                            @for w in report.warnings.iter().take(10) {
                                li { (w) }
                            }
                            @if report.warnings.len() > 10 {
                                li.more { "… and " (report.warnings.len() - 10) " more" }
                            }
                        }
                    }
                }

                // Error details
                @if !report.error_details.is_empty() {
                    section.results-section {
                        h3 { "Errors (" (report.error_details.len()) ")" }
                        ul.error-list {
                            @for e in report.error_details.iter().take(10) {
                                li { (e) }
                            }
                            @if report.error_details.len() > 10 {
                                li.more { "… and " (report.error_details.len() - 10) " more" }
                            }
                        }
                    }
                }

                // Actions
                div.results-actions {
                    a.btn href="/import" { "Import Another" }
                    a.btn.btn-primary href="/stats" { "View Dashboard" }
                }
            }
        },
    )
}

// ── CSS ─────────────────────────────────────────────────────────────

const CSS: &str = r#"
:root {
    --bg: #fafafa; --bg-card: #ffffff; --text: #1a1a2e;
    --text-secondary: #64748b; --border: #e2e8f0;
    --accent: #6366f1; --accent-hover: #4f46e5;
    --success: #10b981; --warning: #f59e0b; --danger: #ef4444;
    --info: #06b6d4;
    --progress-bg: #e2e8f0; --progress-fill: #6366f1;
}
@media (prefers-color-scheme: dark) {
    :root {
        --bg: #0f172a; --bg-card: #1e293b; --text: #e2e8f0;
        --text-secondary: #94a3b8; --border: #334155;
        --accent: #818cf8; --accent-hover: #6366f1;
        --success: #34d399; --warning: #fbbf24; --danger: #f87171;
        --info: #22d3ee;
        --progress-bg: #334155; --progress-fill: #818cf8;
    }
}

* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
                 "Helvetica Neue", Arial, sans-serif;
    background: var(--bg); color: var(--text); line-height: 1.6;
}

/* Header */
.site-header { background: var(--bg-card); border-bottom: 1px solid var(--border); padding: 0.75rem 1.5rem; }
.header-inner { max-width: 1100px; margin: 0 auto; display: flex; align-items: center; justify-content: space-between; }
.logo { font-size: 1.25rem; font-weight: 700; color: var(--text); text-decoration: none; }
.nav-link { color: var(--accent); text-decoration: none; font-weight: 500; }
.nav-link:hover { text-decoration: underline; }
main { max-width: 900px; margin: 0 auto; padding: 1.5rem; }

/* Import page */
.import-page h1 { font-size: 1.75rem; margin-bottom: 0.25rem; }
.subtitle { color: var(--text-secondary); margin-bottom: 1.5rem; }
.import-cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 1rem; }
.import-card {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 10px; padding: 1.5rem;
}
.import-card h2 { font-size: 1.15rem; margin-bottom: 0.5rem; }
.import-card p { font-size: 0.9rem; color: var(--text-secondary); margin-bottom: 1rem; }

/* Forms */
.form-field { margin-bottom: 1rem; }
.form-field label { display: block; font-size: 0.85rem; font-weight: 500; margin-bottom: 0.25rem; }
.form-field input[type="text"],
.form-field input[type="file"] {
    width: 100%; padding: 0.5rem 0.75rem; border: 1px solid var(--border);
    border-radius: 6px; background: var(--bg); color: var(--text); font-size: 0.9rem;
}
.checkbox-field { display: flex; align-items: center; gap: 0.5rem; }
.checkbox-field label { margin-bottom: 0; }
.btn {
    display: inline-block; padding: 0.5rem 1.25rem; border-radius: 6px;
    font-size: 0.9rem; font-weight: 500; cursor: pointer; text-decoration: none;
    border: 1px solid var(--border); background: var(--bg-card); color: var(--text);
    transition: background 0.15s, border-color 0.15s;
}
.btn:hover { border-color: var(--accent); color: var(--accent); }
.btn-primary { background: var(--accent); color: #fff; border-color: var(--accent); }
.btn-primary:hover { background: var(--accent-hover); }
.btn-secondary { margin-left: 0.5rem; }

/* Preview */
.preview-page h1 { font-size: 1.5rem; margin-bottom: 0.25rem; }
.preview-summary, .results-summary {
    display: flex; flex-wrap: wrap; gap: 1rem; margin: 1.5rem 0;
}
.preview-stat, .result-stat {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 10px; padding: 1rem 1.5rem; text-align: center; min-width: 120px;
}
.stat-value { display: block; font-size: 2rem; font-weight: 700; }
.stat-label { display: block; font-size: 0.8rem; color: var(--text-secondary); }
.stat-import .stat-value { color: var(--success); }
.stat-skip .stat-value { color: var(--text-secondary); }
.stat-update .stat-value { color: var(--info); }
.stat-error .stat-value { color: var(--danger); }

.preview-section, .results-section { margin-bottom: 1.5rem; }
.preview-section h3, .results-section h3 {
    font-size: 1rem; margin-bottom: 0.5rem; color: var(--text-secondary);
}
.status-pills { display: flex; flex-wrap: wrap; gap: 0.5rem; }
.status-pill {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 20px; padding: 0.25rem 0.75rem; font-size: 0.85rem;
}

/* Tables */
.sample-table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
.sample-table th { text-align: left; padding: 0.5rem; border-bottom: 2px solid var(--border); color: var(--text-secondary); font-weight: 600; }
.sample-table td { padding: 0.5rem; border-bottom: 1px solid var(--border); }
.row-skipped td { color: var(--text-secondary); }
.row-updated td { color: var(--info); }

.warning-list, .error-list { list-style: none; padding: 0; }
.warning-list li { padding: 0.25rem 0; color: var(--warning); font-size: 0.9rem; }
.error-list li { padding: 0.25rem 0; color: var(--danger); font-size: 0.9rem; }
.more { color: var(--text-secondary); font-style: italic; }

.preview-actions, .results-actions { margin-top: 2rem; display: flex; gap: 0.5rem; }

/* Progress */
.progress-page { text-align: center; }
.progress-page h1 { font-size: 1.5rem; margin-bottom: 1.5rem; }
.progress-container { max-width: 500px; margin: 0 auto 2rem; }
.progress-bar-outer {
    height: 12px; background: var(--progress-bg); border-radius: 6px; overflow: hidden;
}
.progress-bar-inner {
    height: 100%; background: var(--progress-fill); border-radius: 6px;
    transition: width 0.2s; min-width: 2%;
}
.progress-text { margin-top: 0.5rem; color: var(--text-secondary); font-size: 0.9rem; }
.progress-counters {
    display: flex; justify-content: center; gap: 1.5rem; margin-bottom: 1.5rem; flex-wrap: wrap;
}
.counter { text-align: center; }
.counter-value { display: block; font-size: 1.5rem; font-weight: 700; }
.counter-label { font-size: 0.8rem; color: var(--text-secondary); }
.current-title { color: var(--text-secondary); font-size: 0.9rem; min-height: 1.5em; }

/* Import ID */
.import-id-section {
    background: var(--bg-card); border: 1px solid var(--border);
    border-radius: 10px; padding: 1rem 1.25rem; margin-bottom: 1.5rem;
}
.import-id-section code {
    background: var(--bg); padding: 0.15rem 0.4rem; border-radius: 4px;
    font-size: 0.85rem;
}
.undo-hint { font-size: 0.85rem; color: var(--text-secondary); margin-top: 0.25rem; }

/* Responsive */
@media (max-width: 640px) {
    .import-cards { grid-template-columns: 1fr; }
    .preview-summary, .results-summary { flex-direction: column; }
    .progress-counters { gap: 1rem; }
}
"#;
