//! SVG chart generation for the statistics dashboard.
//!
//! All charts use CSS custom properties for theming so they automatically
//! adapt to light/dark mode. Output is raw SVG markup for embedding via
//! `maud::PreEscaped`.

use std::fmt::Write;

use toku_core::{AuthorCount, FormatBreakdown, MonthlyFinished, RatingDistribution, TagCount};

/// HTML-escape user-supplied text for safe embedding in SVG.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ── Rating histogram ────────────────────────────────────────────────

/// Vertical bar chart showing count of books at each rating level (0–10,
/// displayed as 0–5 stars).
pub fn rating_histogram(dist: &RatingDistribution) -> String {
    if dist.total_rated == 0 {
        return empty_state("No rated books yet");
    }

    let width = 460;
    let height = 220;
    let ml = 32;
    let mr = 12;
    let mt = 24;
    let mb = 36;
    let chart_w = width - ml - mr;
    let chart_h = height - mt - mb;
    let bar_count = 11usize;
    let gap = 6;
    let bar_w = (chart_w - gap * (bar_count - 1)) / bar_count;
    let max_val = dist.counts.iter().copied().max().unwrap_or(1).max(1);

    let mut s = String::with_capacity(2048);
    write!(
        s,
        r#"<svg viewBox="0 0 {width} {height}" class="chart" role="img" aria-label="Rating distribution">"#
    )
    .unwrap();
    write!(s, "<title>Rating Distribution</title>").unwrap();

    // Baseline
    write!(
        s,
        r#"<line x1="{ml}" y1="{}" x2="{}" y2="{}" class="chart-axis"/>"#,
        mt + chart_h,
        ml + chart_w,
        mt + chart_h
    )
    .unwrap();

    for (i, &count) in dist.counts.iter().enumerate() {
        let x = ml + i * (bar_w + gap);
        let bar_h = (count * chart_h).checked_div(max_val).unwrap_or(0);
        let y = mt + chart_h - bar_h;

        // Bar
        write!(
            s,
            r#"<rect class="chart-bar" x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="3">"#
        )
        .unwrap();
        write!(s, "<title>{count} book(s) rated {i}/10</title>").unwrap();
        s.push_str("</rect>");

        // Value above bar
        if count > 0 {
            let tx = x + bar_w / 2;
            let ty = y.saturating_sub(5);
            write!(
                s,
                r#"<text class="chart-value" x="{tx}" y="{ty}" text-anchor="middle">{count}</text>"#
            )
            .unwrap();
        }

        // X-axis label (show as star rating)
        let tx = x + bar_w / 2;
        let ty = mt + chart_h + 18;
        let label = if i % 2 == 0 {
            format!("{}", i / 2)
        } else {
            format!("{}.5", i / 2)
        };
        write!(
            s,
            r#"<text class="chart-label" x="{tx}" y="{ty}" text-anchor="middle">{label}</text>"#
        )
        .unwrap();
    }

    // X-axis title
    write!(
        s,
        r#"<text class="chart-axis-title" x="{}" y="{}" text-anchor="middle">★ Rating</text>"#,
        ml + chart_w / 2,
        height - 2
    )
    .unwrap();

    s.push_str("</svg>");
    s
}

// ── Monthly reading pace ────────────────────────────────────────────

/// Vertical bar chart showing books finished per month.
pub fn monthly_pace(months: &[MonthlyFinished]) -> String {
    if months.is_empty() {
        return empty_state("No finished books yet");
    }

    let bar_count = months.len();
    let gap = 4usize;
    let bar_w = 30usize.min(600 / bar_count.max(1));
    let ml = 32;
    let mr = 12;
    let mt = 24;
    let mb = 36;
    let chart_w = bar_count * (bar_w + gap) - gap;
    let width = ml + chart_w + mr;
    let chart_h = 160usize;
    let height = mt + chart_h + mb;
    let max_val = months.iter().map(|m| m.count).max().unwrap_or(1).max(1);

    let mut s = String::with_capacity(2048);
    write!(
        s,
        r#"<svg viewBox="0 0 {width} {height}" class="chart" role="img" aria-label="Monthly reading pace">"#
    )
    .unwrap();
    write!(s, "<title>Monthly Reading Pace</title>").unwrap();

    // Baseline
    write!(
        s,
        r#"<line x1="{ml}" y1="{}" x2="{}" y2="{}" class="chart-axis"/>"#,
        mt + chart_h,
        ml + chart_w,
        mt + chart_h
    )
    .unwrap();

    for (i, m) in months.iter().enumerate() {
        let x = ml + i * (bar_w + gap);
        let bar_h = (m.count * chart_h).checked_div(max_val).unwrap_or(0);
        let y = mt + chart_h - bar_h;
        let month_name = MONTH_NAMES
            .get(m.month.saturating_sub(1) as usize)
            .unwrap_or(&"?");

        write!(
            s,
            r#"<rect class="chart-bar" x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="3">"#
        )
        .unwrap();
        write!(
            s,
            "<title>{month_name} {}: {} book(s)</title>",
            m.year, m.count
        )
        .unwrap();
        s.push_str("</rect>");

        if m.count > 0 {
            let tx = x + bar_w / 2;
            let ty = y.saturating_sub(5);
            write!(
                s,
                r#"<text class="chart-value" x="{tx}" y="{ty}" text-anchor="middle">{}</text>"#,
                m.count
            )
            .unwrap();
        }

        // X-axis label — show month (and year if it changes)
        let tx = x + bar_w / 2;
        let ty = mt + chart_h + 16;
        let show_year = i == 0
            || months
                .get(i.wrapping_sub(1))
                .is_some_and(|p| p.year != m.year);
        let label = if show_year {
            format!("{month_name} '{}", m.year % 100)
        } else {
            (*month_name).to_string()
        };
        write!(
            s,
            r#"<text class="chart-label" x="{tx}" y="{ty}" text-anchor="middle">{label}</text>"#
        )
        .unwrap();
    }

    s.push_str("</svg>");
    s
}

// ── Format breakdown donut ──────────────────────────────────────────

const FORMAT_COLORS: [&str; 3] = ["var(--chart-1)", "var(--chart-2)", "var(--chart-3)"];

/// Donut chart showing physical / ebook / audiobook breakdown.
pub fn format_donut(breakdown: &FormatBreakdown) -> String {
    let segments = [
        ("Physical", breakdown.physical),
        ("Ebook", breakdown.ebook),
        ("Audiobook", breakdown.audiobook),
    ];
    let total: usize = segments.iter().map(|(_, c)| c).sum();

    if total == 0 {
        return empty_state("No books in library");
    }

    let size = 240;
    let cx = size / 2;
    let cy = 100;
    let r = 70usize;
    let stroke_w = 28;
    let circumference = 2.0 * std::f64::consts::PI * r as f64;

    let mut s = String::with_capacity(1024);
    write!(
        s,
        r#"<svg viewBox="0 0 {size} 240" class="chart" role="img" aria-label="Format breakdown">"#
    )
    .unwrap();
    write!(s, "<title>Format Breakdown</title>").unwrap();

    let mut offset = 0.0f64;
    for (i, &(label, count)) in segments.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let frac = count as f64 / total as f64;
        let dash = frac * circumference;
        let gap = circumference - dash;
        let color = FORMAT_COLORS[i];

        write!(
            s,
            r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{color}" stroke-width="{stroke_w}" stroke-dasharray="{dash:.1} {gap:.1}" stroke-dashoffset="{offset:.1}" transform="rotate(-90 {cx} {cy})">"#
        ).unwrap();
        write!(s, "<title>{label}: {count} ({:.0}%)</title>", frac * 100.0).unwrap();
        s.push_str("</circle>");
        offset -= dash;
    }

    // Center label
    write!(
        s,
        r#"<text x="{cx}" y="{}" text-anchor="middle" class="chart-center-value">{total}</text>"#,
        cy - 4
    )
    .unwrap();
    write!(
        s,
        r#"<text x="{cx}" y="{}" text-anchor="middle" class="chart-center-label">books</text>"#,
        cy + 14
    )
    .unwrap();

    // Legend
    let legend_y = cy + r + 36;
    let mut lx = 10usize;
    for (i, &(label, count)) in segments.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let color = FORMAT_COLORS[i];
        write!(
            s,
            r#"<rect x="{lx}" y="{}" width="12" height="12" rx="2" fill="{color}"/>"#,
            legend_y - 10
        )
        .unwrap();
        write!(
            s,
            r#"<text x="{}" y="{legend_y}" class="chart-legend">{label} ({count})</text>"#,
            lx + 16
        )
        .unwrap();
        lx += 16 + label.len() * 7 + format!(" ({count})").len() * 7 + 12;
    }

    s.push_str("</svg>");
    s
}

// ── Horizontal bar chart (tags, authors) ────────────────────────────

/// Horizontal bar chart for tag distribution (top N items).
pub fn tag_bar_chart(tags: &[TagCount], max_items: usize) -> String {
    let items: Vec<(&str, usize)> = tags
        .iter()
        .take(max_items)
        .map(|t| (t.name.as_str(), t.count))
        .collect();
    horizontal_bars(&items, "Tag distribution")
}

/// Horizontal bar chart for top authors.
pub fn author_bar_chart(authors: &[AuthorCount], max_items: usize) -> String {
    let items: Vec<(&str, usize)> = authors
        .iter()
        .take(max_items)
        .map(|a| (a.name.as_str(), a.count))
        .collect();
    horizontal_bars(&items, "Top authors")
}

fn horizontal_bars(items: &[(&str, usize)], title: &str) -> String {
    if items.is_empty() {
        return empty_state(&format!("No {title} data"));
    }

    let max_label_len = items.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let label_w = (max_label_len * 7 + 12).min(140);
    let bar_area = 260;
    let value_w = 40;
    let width = label_w + bar_area + value_w;
    let row_h = 28usize;
    let gap = 4usize;
    let mt = 8;
    let height = mt + items.len() * (row_h + gap);
    let max_val = items.iter().map(|(_, v)| *v).max().unwrap_or(1).max(1);

    let mut s = String::with_capacity(2048);
    write!(
        s,
        r#"<svg viewBox="0 0 {width} {height}" class="chart" role="img" aria-label="{title}">"#
    )
    .unwrap();
    write!(s, "<title>{title}</title>").unwrap();

    for (i, &(label, count)) in items.iter().enumerate() {
        let y = mt + i * (row_h + gap);
        let bar_w = (count * bar_area).checked_div(max_val).unwrap_or(0);

        // Label (right-aligned)
        let truncated = if label.len() > 18 {
            format!("{}…", &label[..17])
        } else {
            label.to_string()
        };
        write!(
            s,
            r#"<text class="chart-h-label" x="{}" y="{}" text-anchor="end" dominant-baseline="middle">{}</text>"#,
            label_w - 4,
            y + row_h / 2,
            esc(&truncated)
        )
        .unwrap();

        // Bar
        write!(
            s,
            r#"<rect class="chart-bar" x="{label_w}" y="{y}" width="{bar_w}" height="{row_h}" rx="3">"#
        )
        .unwrap();
        write!(s, "<title>{}: {count}</title>", esc(label)).unwrap();
        s.push_str("</rect>");

        // Value
        write!(
            s,
            r#"<text class="chart-h-value" x="{}" y="{}" dominant-baseline="middle">{count}</text>"#,
            label_w + bar_w + 6,
            y + row_h / 2
        )
        .unwrap();
    }

    s.push_str("</svg>");
    s
}

// ── Empty state ─────────────────────────────────────────────────────

fn empty_state(message: &str) -> String {
    format!(
        r#"<svg viewBox="0 0 300 80" class="chart chart-empty" role="img" aria-label="{message}"><text x="150" y="45" text-anchor="middle" class="chart-empty-text">{message}</text></svg>"#
    )
}
