//! HTML view rendering using maud.

use maud::{DOCTYPE, Markup, PreEscaped, html};
use toku_core::ReadingStats;

use crate::charts;

/// Wrap content in the base HTML layout.
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
                footer.site-footer {
                    p { "Toku — your private reading tracker" }
                }
            }
        }
    }
}

/// Render the main statistics dashboard.
pub fn dashboard(stats: &ReadingStats, year: Option<i32>, available_years: &[i32]) -> Markup {
    let content = dashboard_content(stats, year, available_years);
    let title = match year {
        Some(y) => format!("{y} Reading Dashboard"),
        None => "Reading Dashboard".to_string(),
    };
    base(&title, content)
}

/// Just the dashboard content (for potential HTMX partial updates).
pub fn dashboard_content(
    stats: &ReadingStats,
    year: Option<i32>,
    available_years: &[i32],
) -> Markup {
    html! {
        div.dashboard {
            // Header with year selector
            div.dashboard-header {
                h1 {
                    @if let Some(y) = year {
                        (y) " Reading Dashboard"
                    } @else {
                        "Reading Dashboard"
                    }
                }
                @if !available_years.is_empty() {
                    nav.year-nav {
                        @let all_cls = if year.is_none() { "year-link active" } else { "year-link" };
                        a class=(all_cls) href="/stats" { "All Time" }
                        @for &y in available_years {
                            @let cls = if year == Some(y) { "year-link active" } else { "year-link" };
                            a class=(cls)
                              href={ "/stats?year=" (y) }
                            { (y) }
                        }
                    }
                }
            }

            // Key metrics
            section.metrics {
                (metric_card("Books Read", &stats.books_read.to_string(), "📖"))
                (metric_card("Currently Reading", &stats.books_reading.to_string(), "📕"))
                (metric_card("Want to Read", &stats.books_want_to_read.to_string(), "📋"))
                (metric_card("Total Pages", &format_number(stats.total_pages_read), "📄"))
                (metric_card(
                    "Avg Rating",
                    &match stats.average_rating_stars {
                        Some(r) => format!("{r:.1}★"),
                        None => "—".to_string(),
                    },
                    "⭐"
                ))
                (metric_card(
                    "Books/Month",
                    &format!("{:.1}", stats.books_per_month),
                    "📅"
                ))
                (metric_card(
                    "Pages/Day",
                    &format!("{:.0}", stats.pages_per_day),
                    "🏃"
                ))
                (metric_card(
                    "Avg Days to Finish",
                    &match stats.avg_days_to_finish {
                        Some(d) => format!("{d:.0}"),
                        None => "—".to_string(),
                    },
                    "⏱️"
                ))
            }

            // Streaks
            section.streaks {
                h2 { "Reading Streaks" }
                div.streak-cards {
                    div.streak-card {
                        span.streak-value { (stats.reading_streaks.current_streak_days) }
                        span.streak-label { "Current streak (days)" }
                    }
                    div.streak-card {
                        span.streak-value { (stats.reading_streaks.longest_streak_days) }
                        span.streak-label { "Longest streak (days)" }
                    }
                    div.streak-card {
                        span.streak-value { (stats.reading_streaks.total_active_days) }
                        span.streak-label { "Total active days" }
                    }
                }
            }

            // Charts row 1: rating + monthly pace
            section.charts-row {
                div.chart-card {
                    h3 { "Rating Distribution" }
                    (PreEscaped(charts::rating_histogram(&stats.rating_distribution)))
                }
                div.chart-card {
                    h3 { "Monthly Pace" }
                    (PreEscaped(charts::monthly_pace(&stats.monthly_finished)))
                }
            }

            // Charts row 2: format donut + tags
            section.charts-row {
                div.chart-card {
                    h3 { "Format Breakdown" }
                    (PreEscaped(charts::format_donut(&stats.format_breakdown)))
                }
                div.chart-card {
                    h3 { "Top Tags" }
                    (PreEscaped(charts::tag_bar_chart(&stats.tag_distribution, 10)))
                }
            }

            // Charts row 3: authors
            section.charts-row {
                div.chart-card {
                    h3 { "Top Authors" }
                    p.chart-subtitle {
                        (stats.author_stats.unique_count) " unique author(s)"
                    }
                    (PreEscaped(charts::author_bar_chart(&stats.author_stats.top_authors, 10)))
                }

                // Shortest / Longest
                div.chart-card {
                    h3 { "Extremes" }
                    @if let Some(ref shortest) = stats.shortest_book {
                        p.extremes-item {
                            span.extremes-label { "Shortest: " }
                            span.extremes-value { (shortest.title) }
                            span.extremes-pages { " (" (shortest.page_count) " pp)" }
                        }
                    }
                    @if let Some(ref longest) = stats.longest_book {
                        p.extremes-item {
                            span.extremes-label { "Longest: " }
                            span.extremes-value { (longest.title) }
                            span.extremes-pages { " (" (longest.page_count) " pp)" }
                        }
                    }
                    @if stats.shortest_book.is_none() && stats.longest_book.is_none() {
                        p.chart-empty-text { "No page count data" }
                    }
                }
            }

            // Currently reading
            @if !stats.currently_reading.is_empty() {
                section.currently-reading {
                    h2 { "Currently Reading" }
                    div.reading-list {
                        @for book in &stats.currently_reading {
                            div.reading-item {
                                div.reading-info {
                                    span.reading-title { (book.title) }
                                    span.reading-author { (book.author) }
                                }
                                @if let Some(pct) = book.percent {
                                    div.progress-bar {
                                        div.progress-fill style=(format!("width:{pct:.0}%")) {}
                                    }
                                    span.progress-text {
                                        @if let (Some(page), Some(total)) = (book.latest_page, book.total_pages) {
                                            (page) "/" (total) " (" (format!("{pct:.0}")) "%)"
                                        } @else {
                                            (format!("{pct:.0}")) "%"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Reading speed
            @if let Some(speed) = stats.reading_speed_pages_per_hour {
                section.speed-note {
                    p {
                        "Reading speed: " strong { (format!("{speed:.0}")) " pages/hour" }
                    }
                }
            }
        }
    }
}

/// Render the yearly wrap-up page.
pub fn yearly_wrap(stats: &ReadingStats, year: i32) -> Markup {
    let content = html! {
        div.wrap {
            div.wrap-hero {
                h1 { (year) " Year in Reading" }
                p.wrap-subtitle { "Your reading journey this year" }
            }

            // Headline stats
            section.wrap-highlights {
                div.wrap-stat {
                    span.wrap-big { (stats.books_read) }
                    span { "books finished" }
                }
                div.wrap-stat {
                    span.wrap-big { (format_number(stats.total_pages_read)) }
                    span { "pages read" }
                }
                div.wrap-stat {
                    span.wrap-big {
                        @match stats.average_rating_stars {
                            Some(r) => { (format!("{r:.1}")) "★" },
                            None => { "—" },
                        }
                    }
                    span { "average rating" }
                }
                div.wrap-stat {
                    span.wrap-big { (format!("{:.1}", stats.books_per_month)) }
                    span { "books/month" }
                }
            }

            // Streaks
            section.wrap-section {
                h2 { "Streaks" }
                div.streak-cards {
                    div.streak-card {
                        span.streak-value { (stats.reading_streaks.longest_streak_days) }
                        span.streak-label { "Longest streak" }
                    }
                    div.streak-card {
                        span.streak-value { (stats.reading_streaks.total_active_days) }
                        span.streak-label { "Active days" }
                    }
                }
            }

            // Monthly pace chart
            @if !stats.monthly_finished.is_empty() {
                section.wrap-section {
                    h2 { "Month by Month" }
                    div.chart-card {
                        (PreEscaped(charts::monthly_pace(&stats.monthly_finished)))
                    }
                }
            }

            // Rating distribution
            section.wrap-section {
                h2 { "Your Ratings" }
                div.chart-card {
                    (PreEscaped(charts::rating_histogram(&stats.rating_distribution)))
                }
            }

            // Format + Tags
            section.charts-row {
                div.chart-card {
                    h3 { "Formats" }
                    (PreEscaped(charts::format_donut(&stats.format_breakdown)))
                }
                div.chart-card {
                    h3 { "Top Tags" }
                    (PreEscaped(charts::tag_bar_chart(&stats.tag_distribution, 10)))
                }
            }

            // Authors
            section.wrap-section {
                h2 { "Authors" }
                p { (stats.author_stats.unique_count) " unique author(s) this year" }
                div.chart-card {
                    (PreEscaped(charts::author_bar_chart(&stats.author_stats.top_authors, 10)))
                }
            }

            // Extremes
            section.wrap-section {
                h2 { "Extremes" }
                div.wrap-extremes {
                    @if let Some(ref shortest) = stats.shortest_book {
                        div.wrap-extreme {
                            span.extremes-label { "Shortest book" }
                            span.extremes-value { (shortest.title) }
                            span.extremes-pages { (shortest.page_count) " pages" }
                        }
                    }
                    @if let Some(ref longest) = stats.longest_book {
                        div.wrap-extreme {
                            span.extremes-label { "Longest book" }
                            span.extremes-value { (longest.title) }
                            span.extremes-pages { (longest.page_count) " pages" }
                        }
                    }
                }
            }

            div.wrap-footer {
                a href="/stats" { "← Back to Dashboard" }
            }
        }
    };

    base(&format!("{year} Year in Reading"), content)
}

// ── Helpers ─────────────────────────────────────────────────────────

fn metric_card(label: &str, value: &str, icon: &str) -> Markup {
    html! {
        div.metric-card {
            span.metric-icon { (icon) }
            span.metric-value { (value) }
            span.metric-label { (label) }
        }
    }
}

fn format_number(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

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
    --chart-1: #6366f1;
    --chart-2: #06b6d4;
    --chart-3: #10b981;
    --chart-bar: #6366f1;
    --chart-bar-hover: #4f46e5;
    --chart-axis: #cbd5e1;
    --chart-text: #64748b;
    --chart-value: #1a1a2e;
    --streak-bg: #eef2ff;
    --streak-text: #4338ca;
    --progress-bg: #e2e8f0;
    --progress-fill: #6366f1;
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
        --chart-1: #818cf8;
        --chart-2: #22d3ee;
        --chart-3: #34d399;
        --chart-bar: #818cf8;
        --chart-bar-hover: #a5b4fc;
        --chart-axis: #475569;
        --chart-text: #94a3b8;
        --chart-value: #e2e8f0;
        --streak-bg: #312e81;
        --streak-text: #c7d2fe;
        --progress-bg: #334155;
        --progress-fill: #818cf8;
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

/* Header */
.site-header {
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
    padding: 0.75rem 1.5rem;
}
.header-inner {
    max-width: 1100px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
}
.logo {
    font-size: 1.25rem;
    font-weight: 700;
    color: var(--text);
    text-decoration: none;
}
.nav-link {
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
}
.nav-link:hover { text-decoration: underline; }

/* Footer */
.site-footer {
    text-align: center;
    padding: 2rem 1rem;
    color: var(--text-secondary);
    font-size: 0.85rem;
}

/* Main */
main {
    max-width: 1100px;
    margin: 0 auto;
    padding: 1.5rem;
}

/* Dashboard */
.dashboard-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 1rem;
    margin-bottom: 1.5rem;
}
.dashboard-header h1 { font-size: 1.75rem; }

/* Year navigation */
.year-nav {
    display: flex;
    gap: 0.25rem;
    flex-wrap: wrap;
}
.year-link {
    display: inline-block;
    padding: 0.3rem 0.75rem;
    border-radius: 6px;
    text-decoration: none;
    font-size: 0.85rem;
    color: var(--text-secondary);
    background: var(--bg-card);
    border: 1px solid var(--border);
    transition: background 0.15s, color 0.15s;
}
.year-link:hover { color: var(--accent); border-color: var(--accent); }
.year-link.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
}

/* Metric cards */
.metrics {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 0.75rem;
    margin-bottom: 2rem;
}
.metric-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
}
.metric-icon { font-size: 1.5rem; margin-bottom: 0.25rem; }
.metric-value { font-size: 1.5rem; font-weight: 700; }
.metric-label { font-size: 0.8rem; color: var(--text-secondary); }

/* Streaks */
.streaks { margin-bottom: 2rem; }
.streaks h2 { margin-bottom: 0.75rem; }
.streak-cards {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
}
.streak-card {
    background: var(--streak-bg);
    color: var(--streak-text);
    border-radius: 10px;
    padding: 1rem 1.5rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 140px;
}
.streak-value { font-size: 2rem; font-weight: 700; }
.streak-label { font-size: 0.8rem; }

/* Chart layout */
.charts-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: 1rem;
    margin-bottom: 1.5rem;
}
.chart-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem;
}
.chart-card h3 {
    font-size: 1rem;
    margin-bottom: 0.75rem;
    color: var(--text-secondary);
}
.chart-subtitle {
    font-size: 0.85rem;
    color: var(--text-secondary);
    margin-bottom: 0.5rem;
}

/* SVG chart styles */
.chart { width: 100%; height: auto; }
.chart-bar { fill: var(--chart-bar); transition: fill 0.15s; }
.chart-bar:hover { fill: var(--chart-bar-hover); }
.chart-axis { stroke: var(--chart-axis); stroke-width: 1; }
.chart-label { font-size: 11px; fill: var(--chart-text); }
.chart-value { font-size: 11px; fill: var(--chart-value); font-weight: 600; }
.chart-h-label { font-size: 12px; fill: var(--chart-text); }
.chart-h-value { font-size: 12px; fill: var(--chart-value); font-weight: 600; }
.chart-center-value { font-size: 24px; font-weight: 700; fill: var(--text); }
.chart-center-label { font-size: 12px; fill: var(--text-secondary); }
.chart-legend { font-size: 12px; fill: var(--text-secondary); }
.chart-empty-text { font-size: 14px; fill: var(--text-secondary); }
.chart-axis-title { font-size: 11px; fill: var(--text-secondary); }

/* Currently reading */
.currently-reading { margin-bottom: 2rem; }
.currently-reading h2 { margin-bottom: 0.75rem; }
.reading-list { display: flex; flex-direction: column; gap: 0.75rem; }
.reading-item {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem;
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
}
.reading-info { flex: 1; min-width: 200px; }
.reading-title { display: block; font-weight: 600; }
.reading-author { display: block; font-size: 0.85rem; color: var(--text-secondary); }
.progress-bar {
    width: 120px;
    height: 8px;
    background: var(--progress-bg);
    border-radius: 4px;
    overflow: hidden;
}
.progress-fill {
    height: 100%;
    background: var(--progress-fill);
    border-radius: 4px;
    transition: width 0.3s;
}
.progress-text { font-size: 0.8rem; color: var(--text-secondary); }

/* Extremes */
.extremes-item { margin-bottom: 0.5rem; }
.extremes-label { color: var(--text-secondary); font-size: 0.85rem; }
.extremes-value { font-weight: 600; }
.extremes-pages { font-size: 0.85rem; color: var(--text-secondary); }

/* Speed note */
.speed-note {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem 1.25rem;
    margin-bottom: 2rem;
    color: var(--text-secondary);
}

/* Wrap (yearly) */
.wrap-hero {
    text-align: center;
    padding: 2rem 0;
}
.wrap-hero h1 { font-size: 2.25rem; }
.wrap-subtitle { color: var(--text-secondary); font-size: 1.1rem; }

.wrap-highlights {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 1rem;
    margin-bottom: 2.5rem;
}
.wrap-stat {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.5rem;
    text-align: center;
}
.wrap-big {
    display: block;
    font-size: 2.5rem;
    font-weight: 800;
    color: var(--accent);
}

.wrap-section { margin-bottom: 2rem; }
.wrap-section h2 { margin-bottom: 0.75rem; }

.wrap-extremes { display: flex; flex-wrap: wrap; gap: 1.5rem; }
.wrap-extreme {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
}
.wrap-extreme .extremes-label { font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.05em; }
.wrap-extreme .extremes-value { font-size: 1.1rem; }
.wrap-extreme .extremes-pages { font-size: 0.85rem; }

.wrap-footer {
    text-align: center;
    padding: 2rem 0;
}
.wrap-footer a {
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
}
.wrap-footer a:hover { text-decoration: underline; }

/* Responsive */
@media (max-width: 640px) {
    .dashboard-header { flex-direction: column; }
    .metrics { grid-template-columns: repeat(2, 1fr); }
    .charts-row { grid-template-columns: 1fr; }
    .streak-cards { flex-direction: column; }
}
"#;
