use std::io::{self, IsTerminal, Stderr};
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Padding, Paragraph},
};
use toku_db::Database;
use toku_import::{
    GoodreadsImportOptions, ImportEvent, ImportObserver, ImportReport, RowOutcome,
    StorygraphImportOptions,
};

use crate::OutputFormat;

/// RAII guard that restores the terminal on drop, even if the import panics.
struct TerminalGuard {
    terminal: ratatui::Terminal<CrosstermBackend<Stderr>>,
    active: bool,
}

impl TerminalGuard {
    fn init() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stderr = io::stderr();
        execute!(stderr, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(io::stderr());
        let terminal =
            ratatui::Terminal::new(backend).context("failed to create ratatui terminal")?;
        Ok(Self {
            terminal,
            active: true,
        })
    }

    fn restore(&mut self) {
        if self.active {
            self.active = false;
            let _ = disable_raw_mode();
            let _ = execute!(io::stderr(), LeaveAlternateScreen);
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

// ── Progress State ──────────────────────────────────────────────────────────

const ACTIVITY_LOG_SIZE: usize = 8;

struct ActivityEntry {
    title: String,
    author: String,
    status: String,
    outcome: RowOutcome,
}

struct ImportProgressState {
    current: usize,
    total: usize,
    imported: usize,
    updated: usize,
    skipped: usize,
    errors: usize,
    recent: Vec<ActivityEntry>,
    dry_run: bool,
}

impl ImportProgressState {
    fn new(total: usize, dry_run: bool) -> Self {
        Self {
            current: 0,
            total,
            imported: 0,
            updated: 0,
            skipped: 0,
            errors: 0,
            recent: Vec::with_capacity(ACTIVITY_LOG_SIZE),
            dry_run,
        }
    }

    fn update(&mut self, event: &ImportEvent) {
        self.current = event.row;
        match &event.outcome {
            RowOutcome::Imported => self.imported += 1,
            RowOutcome::Updated => self.updated += 1,
            RowOutcome::Skipped => self.skipped += 1,
            RowOutcome::Error(_) => self.errors += 1,
        }
        self.recent.push(ActivityEntry {
            title: event.title.clone(),
            author: event.author.clone(),
            status: event.status.clone(),
            outcome: event.outcome.clone(),
        });
        if self.recent.len() > ACTIVITY_LOG_SIZE {
            self.recent.remove(0);
        }
    }

    fn progress_ratio(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.current as f64 / self.total as f64
        }
    }

    fn progress_pct(&self) -> u16 {
        (self.progress_ratio() * 100.0).round().min(100.0) as u16
    }
}

// ── Observer Implementation ─────────────────────────────────────────────────

struct RatatuiImportObserver {
    guard: TerminalGuard,
    state: ImportProgressState,
}

impl RatatuiImportObserver {
    fn new(total: usize, dry_run: bool) -> Result<Self> {
        let guard = TerminalGuard::init()?;
        let state = ImportProgressState::new(total, dry_run);
        Ok(Self { guard, state })
    }

    fn restore(mut self) {
        self.guard.restore();
    }
}

impl ImportObserver for RatatuiImportObserver {
    fn on_event(&mut self, event: &ImportEvent) -> Result<(), toku_import::ImportError> {
        self.state.update(event);
        self.guard
            .terminal
            .draw(|frame| render_import_progress(frame, &self.state))
            .map_err(|e| toku_import::ImportError::Other(format!("render error: {e}")))?;
        Ok(())
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render_import_progress(frame: &mut Frame, state: &ImportProgressState) {
    let area = frame.area();

    // Vertical layout: header + progress bar + stats + activity log
    let chunks = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Length(3), // progress bar
        Constraint::Length(3), // stats
        Constraint::Min(4),    // activity log
    ])
    .split(area);

    render_header(frame, chunks[0], state);
    render_progress_bar(frame, chunks[1], state);
    render_stats(frame, chunks[2], state);
    render_activity_log(frame, chunks[3], state);
}

fn render_header(frame: &mut Frame, area: Rect, state: &ImportProgressState) {
    let title = if state.dry_run {
        " 📚 Dry Run Preview — Goodreads Import "
    } else {
        " 📚 Importing from Goodreads "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(block, area);
}

fn render_progress_bar(frame: &mut Frame, area: Rect, state: &ImportProgressState) {
    let label = format!(
        "{}/{} books  {}%",
        state.current,
        state.total,
        state.progress_pct()
    );
    let gauge = Gauge::default()
        .block(Block::default().padding(Padding::horizontal(2)))
        .gauge_style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(state.progress_ratio())
        .label(label);
    frame.render_widget(gauge, area);
}

fn render_stats(frame: &mut Frame, area: Rect, state: &ImportProgressState) {
    let imported_style = Style::default().fg(Color::Green).bold();
    let updated_style = Style::default().fg(Color::Cyan).bold();
    let skipped_style = Style::default().fg(Color::Yellow).bold();
    let error_style = Style::default().fg(Color::Red).bold();
    let label_style = Style::default().fg(Color::Gray);

    let line = Line::from(vec![
        Span::raw("    "),
        Span::styled("✓ ", imported_style),
        Span::styled("Imported ", label_style),
        Span::styled(format!("{:<6}", state.imported), imported_style),
        Span::raw("   "),
        Span::styled("⟳ ", updated_style),
        Span::styled("Updated ", label_style),
        Span::styled(format!("{:<6}", state.updated), updated_style),
        Span::raw("   "),
        Span::styled("↷ ", skipped_style),
        Span::styled("Duplicates ", label_style),
        Span::styled(format!("{:<6}", state.skipped), skipped_style),
        Span::raw("   "),
        Span::styled("✗ ", error_style),
        Span::styled("Errors ", label_style),
        Span::styled(format!("{}", state.errors), error_style),
    ]);

    let paragraph = Paragraph::new(line).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(paragraph, area);
}

fn render_activity_log(frame: &mut Frame, area: Rect, state: &ImportProgressState) {
    let block = Block::default()
        .title(" Recent ")
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::new(2, 2, 0, 0));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.recent.is_empty() {
        return;
    }

    let lines: Vec<Line> = state
        .recent
        .iter()
        .map(|entry| {
            let (icon, icon_style) = match &entry.outcome {
                RowOutcome::Imported => ("✓ ", Style::default().fg(Color::Green)),
                RowOutcome::Updated => ("⟳ ", Style::default().fg(Color::Cyan)),
                RowOutcome::Skipped => ("↷ ", Style::default().fg(Color::Yellow)),
                RowOutcome::Error(_) => ("✗ ", Style::default().fg(Color::Red)),
            };

            let title = truncate_str(&entry.title, 40);
            let author = if entry.author.is_empty() {
                String::new()
            } else {
                format!(" — {}", truncate_str(&entry.author, 25))
            };

            let right_label = match &entry.outcome {
                RowOutcome::Imported => entry.status.clone(),
                RowOutcome::Updated => "(tags updated)".to_string(),
                RowOutcome::Skipped => "(duplicate)".to_string(),
                RowOutcome::Error(e) => truncate_str(e, 30),
            };

            let right_style = match &entry.outcome {
                RowOutcome::Imported => Style::default().fg(Color::DarkGray),
                RowOutcome::Updated => Style::default().fg(Color::Cyan),
                RowOutcome::Skipped => Style::default().fg(Color::Yellow),
                RowOutcome::Error(_) => Style::default().fg(Color::Red),
            };

            Line::from(vec![
                Span::styled(icon, icon_style),
                Span::styled(title, Style::default().fg(Color::White)),
                Span::styled(author, Style::default().fg(Color::Gray)),
                Span::raw("  "),
                Span::styled(right_label, right_style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Truncate a string to a maximum display width, using character boundaries.
pub(crate) fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Run a Goodreads import with a ratatui progress UI (if stderr is a TTY and
/// format is table), or fall back to a plain summary.
pub fn run_goodreads_import(
    db: &Database,
    path: &Path,
    dry_run: bool,
    format: &OutputFormat,
) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }

    let opts = GoodreadsImportOptions { dry_run };
    let use_tui = matches!(format, OutputFormat::Table) && io::stderr().is_terminal();

    if use_tui {
        run_with_tui(db, path, &opts, format)
    } else {
        run_without_tui(db, path, &opts, format)
    }
}

/// Run a StoryGraph import with the same UI as Goodreads import.
pub fn run_storygraph_import(
    db: &Database,
    path: &Path,
    dry_run: bool,
    format: &OutputFormat,
) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }

    let opts = StorygraphImportOptions { dry_run };
    let use_tui = matches!(format, OutputFormat::Table) && io::stderr().is_terminal();

    if use_tui {
        let total = count_csv_rows_quick(path)?;
        let mut observer =
            RatatuiImportObserver::new(total, dry_run).context("failed to initialize TUI")?;

        let result = toku_import::import_storygraph(db, path, &opts, Some(&mut observer));
        observer.restore();

        let report = result.context("StoryGraph import failed")?;
        print_import_summary(&report, format, dry_run);
    } else {
        let report = toku_import::import_storygraph(db, path, &opts, None)
            .context("StoryGraph import failed")?;
        print_import_summary(&report, format, dry_run);
    }

    Ok(())
}

fn run_with_tui(
    db: &Database,
    path: &Path,
    opts: &GoodreadsImportOptions,
    format: &OutputFormat,
) -> Result<()> {
    // Pre-count by the observer (it will be counted inside import_goodreads)
    // but we need the count to initialize the observer.
    let total = count_csv_rows_quick(path)?;
    let mut observer =
        RatatuiImportObserver::new(total, opts.dry_run).context("failed to initialize TUI")?;

    let result = toku_import::import_goodreads(db, path, opts, Some(&mut observer));

    // Always restore terminal before printing anything
    observer.restore();

    let report = result.context("Goodreads import failed")?;
    print_import_summary(&report, format, opts.dry_run);
    Ok(())
}

fn run_without_tui(
    db: &Database,
    path: &Path,
    opts: &GoodreadsImportOptions,
    format: &OutputFormat,
) -> Result<()> {
    let report =
        toku_import::import_goodreads(db, path, opts, None).context("Goodreads import failed")?;
    print_import_summary(&report, format, opts.dry_run);
    Ok(())
}

/// Quick CSV row count (using csv crate to handle embedded newlines).
fn count_csv_rows_quick(path: &Path) -> Result<usize> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(path)
        .context("failed to read CSV for row count")?;
    Ok(rdr.records().count())
}

// ── Summary Printing ────────────────────────────────────────────────────────

fn print_import_summary(report: &ImportReport, format: &OutputFormat, dry_run: bool) {
    match format {
        OutputFormat::Json => print_summary_json(report, dry_run),
        OutputFormat::Csv => print_summary_csv(report),
        OutputFormat::Table => print_summary_table(report, dry_run),
    }
}

fn print_summary_table(report: &ImportReport, dry_run: bool) {
    eprintln!();
    if dry_run {
        eprintln!("  📋 Dry run complete (no changes made)\n");
    } else if report.errors == 0 {
        eprintln!("  ✓ Goodreads import complete\n");
    } else {
        eprintln!("  ⚠ Goodreads import completed with errors\n");
    }

    // Main stats
    let status_label = if dry_run { "Would import" } else { "Imported" };
    eprintln!("    {:<14} {:>4} books", status_label, report.imported);
    if report.updated > 0 {
        let update_label = if dry_run { "Would update" } else { "Updated" };
        eprintln!(
            "    {:<14} {:>4} (tags added to existing books)",
            update_label, report.updated
        );
    }
    if report.skipped > 0 {
        eprintln!(
            "    {:<14} {:>4} (already in library)",
            "Duplicates", report.skipped
        );
    }
    if report.errors > 0 {
        eprintln!("    {:<14} {:>4} rows", "Errors", report.errors);
    }
    eprintln!("    {:<14} {:>4}", "Total rows", report.total_rows);

    // Status breakdown
    if !report.status_counts.is_empty() {
        eprintln!("\n  Status breakdown:");
        // Sort for consistent output
        let mut counts: Vec<_> = report.status_counts.iter().collect();
        counts.sort_by(|a, b| b.1.cmp(a.1));
        for (status, count) in counts {
            let label = match status.as_str() {
                "read" => "Read",
                "reading" => "Currently Reading",
                "want-to-read" => "Want to Read",
                "abandoned" => "Abandoned",
                "on-hold" => "On Hold",
                other => other,
            };
            eprintln!("    {:<20} {:>4}", label, count);
        }
    }

    // Updated books sample
    if !report.updated_samples.is_empty() {
        eprintln!("\n  Tags updated on existing books:");
        for s in &report.updated_samples {
            let author = if s.author.is_empty() {
                String::new()
            } else {
                format!(" — {}", s.author)
            };
            eprintln!("    ⟳ {}{}", truncate_str(&s.title, 50), author);
        }
        if report.updated > report.updated_samples.len() {
            eprintln!(
                "    ... and {} more",
                report.updated - report.updated_samples.len()
            );
        }
    }

    // Skipped books sample
    if !report.skipped_samples.is_empty() {
        eprintln!("\n  Duplicates skipped:");
        for s in &report.skipped_samples {
            let author = if s.author.is_empty() {
                String::new()
            } else {
                format!(" — {}", s.author)
            };
            eprintln!("    ↷ {}{}", truncate_str(&s.title, 50), author);
        }
        if report.skipped > report.skipped_samples.len() {
            eprintln!(
                "    ... and {} more",
                report.skipped - report.skipped_samples.len()
            );
        }
    }

    // Errors
    if !report.error_details.is_empty() {
        eprintln!("\n  Errors:");
        for e in &report.error_details {
            eprintln!("    ✗ {e}");
        }
    }

    // Import ID
    if let Some(ref id) = report.import_id {
        eprintln!("\n  Import ID: {id}");
        eprintln!("  To undo:   toku import undo {id}");
    }

    eprintln!();
}

fn print_summary_json(report: &ImportReport, dry_run: bool) {
    #[derive(serde::Serialize)]
    struct JsonReport {
        dry_run: bool,
        total_rows: usize,
        imported: usize,
        updated: usize,
        skipped: usize,
        errors: usize,
        import_id: Option<String>,
        error_details: Vec<String>,
        updated_books: Vec<JsonRowSummary>,
        skipped_books: Vec<JsonRowSummary>,
        imported_books: Vec<JsonRowSummary>,
        status_counts: std::collections::HashMap<String, usize>,
    }
    #[derive(serde::Serialize)]
    struct JsonRowSummary {
        title: String,
        author: String,
        status: String,
    }

    let out = JsonReport {
        dry_run,
        total_rows: report.total_rows,
        imported: report.imported,
        updated: report.updated,
        skipped: report.skipped,
        errors: report.errors,
        import_id: report.import_id.clone(),
        error_details: report.error_details.clone(),
        updated_books: report
            .updated_samples
            .iter()
            .map(|s| JsonRowSummary {
                title: s.title.clone(),
                author: s.author.clone(),
                status: s.status.clone(),
            })
            .collect(),
        skipped_books: report
            .skipped_samples
            .iter()
            .map(|s| JsonRowSummary {
                title: s.title.clone(),
                author: s.author.clone(),
                status: s.status.clone(),
            })
            .collect(),
        imported_books: report
            .imported_samples
            .iter()
            .map(|s| JsonRowSummary {
                title: s.title.clone(),
                author: s.author.clone(),
                status: s.status.clone(),
            })
            .collect(),
        status_counts: report.status_counts.clone(),
    };
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

fn print_summary_csv(report: &ImportReport) {
    println!("total_rows,imported,updated,skipped,errors,import_id");
    println!(
        "{},{},{},{},{},{}",
        report.total_rows,
        report.imported,
        report.updated,
        report.skipped,
        report.errors,
        report.import_id.as_deref().unwrap_or(""),
    );
}
