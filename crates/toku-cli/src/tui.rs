//! Interactive TUI browser for the Toku book library.
//!
//! Split-pane layout: left = scrollable book list, right = book details.
//! Supports filtering by status and tag via a popup overlay.

use std::io::{self, IsTerminal};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::prelude::*;
use ratatui::widgets::*;
use toku_core::{BookFormat, ContributorRole, ReadingStatus};
use toku_db::BookRepository;

use crate::import_ui::truncate_str;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Filter {
    All,
    Status(ReadingStatus),
    Tag(String),
}

impl std::fmt::Display for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Filter::All => write!(f, "All Books"),
            Filter::Status(s) => write!(f, "Status: {s}"),
            Filter::Tag(n) => write!(f, "Tag: {n}"),
        }
    }
}

enum Mode {
    Normal,
    FilterPopup,
}

struct BookDetail {
    authors: Vec<String>,
    isbns: Vec<String>,
    tags: Vec<String>,
}

struct FilterItem {
    label: String,
    filter: Filter,
    selectable: bool,
}

// ── Terminal guard (RAII) ───────────────────────────────────────────────────

/// Restores the terminal on drop — protects against panics leaving the
/// terminal in raw/alternate-screen mode.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
    }
}

// ── App state ───────────────────────────────────────────────────────────────

struct App<'a> {
    repo: &'a BookRepository<'a>,

    // Data
    books: Vec<toku_core::Book>,
    tags: Vec<(toku_core::Tag, i64)>,

    // Active filter
    active_filter: Filter,

    // Book list state
    list_state: ListState,

    // Detail cache (loaded in event handler, read in draw)
    detail: Option<BookDetail>,
    detail_idx: Option<usize>,

    // Filter popup
    mode: Mode,
    filter_items: Vec<FilterItem>,
    filter_state: ListState,

    running: bool,
}

impl<'a> App<'a> {
    fn new(repo: &'a BookRepository<'a>) -> Result<Self> {
        let books = repo.list_books().map_err(|e| anyhow::anyhow!("{e}"))?;
        let tags = repo
            .list_tags_with_counts()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut app = Self {
            repo,
            books,
            tags,
            active_filter: Filter::All,
            list_state: ListState::default(),
            detail: None,
            detail_idx: None,
            mode: Mode::Normal,
            filter_items: Vec::new(),
            filter_state: ListState::default(),
            running: true,
        };

        if !app.books.is_empty() {
            app.list_state.select(Some(0));
            app.ensure_detail_loaded();
        }

        Ok(app)
    }

    // ── Data loading ────────────────────────────────────────────────────

    fn refresh_books(&mut self) {
        let result = match &self.active_filter {
            Filter::All => self.repo.list_books(),
            Filter::Status(s) => {
                let target = *s;
                self.repo
                    .list_books()
                    .map(|books| books.into_iter().filter(|b| b.status == target).collect())
            }
            Filter::Tag(name) => self.repo.list_books_by_tag(name),
        };

        self.books = result.unwrap_or_default();
        self.detail = None;
        self.detail_idx = None;

        if self.books.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
            self.ensure_detail_loaded();
        }
    }

    fn ensure_detail_loaded(&mut self) {
        let idx = match self.list_state.selected() {
            Some(i) if i < self.books.len() => i,
            _ => return,
        };

        if self.detail_idx == Some(idx) {
            return;
        }

        // Copy the ID to release the borrow on self.books
        let book_id = self.books[idx].id;

        let authors = self
            .repo
            .get_book_authors(&book_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(a, ba)| {
                if ba.role == ContributorRole::Author {
                    a.name
                } else {
                    format!("{} ({})", a.name, ba.role)
                }
            })
            .collect();

        let isbns = self.repo.get_book_isbns(&book_id).unwrap_or_default();

        let tags = self
            .repo
            .get_book_tags(&book_id)
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.name)
            .collect();

        self.detail = Some(BookDetail {
            authors,
            isbns,
            tags,
        });
        self.detail_idx = Some(idx);
    }

    // ── Event handling ──────────────────────────────────────────────────

    fn handle_key(&mut self, key: event::KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Ctrl+C always quits
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.running = false;
            return;
        }

        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::FilterPopup => self.handle_filter_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => {
                let page = terminal::size()
                    .map(|(_, h)| h.saturating_sub(6) as i32)
                    .unwrap_or(20);
                self.move_selection(page);
            }
            KeyCode::PageUp => {
                let page = terminal::size()
                    .map(|(_, h)| h.saturating_sub(6) as i32)
                    .unwrap_or(20);
                self.move_selection(-page);
            }
            KeyCode::Home if !self.books.is_empty() => {
                self.list_state.select(Some(0));
                self.ensure_detail_loaded();
            }
            KeyCode::End if !self.books.is_empty() => {
                self.list_state.select(Some(self.books.len() - 1));
                self.ensure_detail_loaded();
            }
            KeyCode::Char('f') => self.open_filter_popup(),
            _ => {}
        }
    }

    fn handle_filter_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_filter_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_filter_selection(-1),
            KeyCode::Enter => {
                if let Some(i) = self.filter_state.selected()
                    && let Some(item) = self.filter_items.get(i)
                    && item.selectable
                {
                    self.active_filter = item.filter.clone();
                    self.refresh_books();
                    self.mode = Mode::Normal;
                }
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.books.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let len = self.books.len() as i32;
        let next = (current + delta).clamp(0, len - 1) as usize;
        self.list_state.select(Some(next));
        self.ensure_detail_loaded();
    }

    fn move_filter_selection(&mut self, delta: i32) {
        let len = self.filter_items.len();
        if len == 0 {
            return;
        }

        let current = self.filter_state.selected().unwrap_or(0) as i32;
        let mut next = (current + delta).rem_euclid(len as i32) as usize;

        // Skip non-selectable header items
        let mut attempts = 0;
        while !self.filter_items[next].selectable && attempts < len {
            next = (next as i32 + delta.signum()).rem_euclid(len as i32) as usize;
            attempts += 1;
        }

        self.filter_state.select(Some(next));
    }

    fn open_filter_popup(&mut self) {
        let mut items = Vec::new();

        items.push(FilterItem {
            label: "  All Books".to_string(),
            filter: Filter::All,
            selectable: true,
        });

        // ── Status section
        items.push(FilterItem {
            label: "── Status ──".to_string(),
            filter: Filter::All,
            selectable: false,
        });
        for status in [
            ReadingStatus::Reading,
            ReadingStatus::Read,
            ReadingStatus::WantToRead,
            ReadingStatus::OnHold,
            ReadingStatus::Abandoned,
        ] {
            let label = match status {
                ReadingStatus::Reading => "  ◉ Reading",
                ReadingStatus::Read => "  ✓ Read",
                ReadingStatus::WantToRead => "  ○ Want to Read",
                ReadingStatus::OnHold => "  ⏸ On Hold",
                ReadingStatus::Abandoned => "  ✗ Abandoned",
            };
            items.push(FilterItem {
                label: label.to_string(),
                filter: Filter::Status(status),
                selectable: true,
            });
        }

        // ── Tags section
        if !self.tags.is_empty() {
            items.push(FilterItem {
                label: "── Tags ──".to_string(),
                filter: Filter::All,
                selectable: false,
            });
            for (tag, count) in &self.tags {
                items.push(FilterItem {
                    label: format!("  🏷 {} ({count})", tag.name),
                    filter: Filter::Tag(tag.name.clone()),
                    selectable: true,
                });
            }
        }

        self.filter_items = items;
        self.filter_state = ListState::default();

        // Select the first selectable item
        if let Some(pos) = self.filter_items.iter().position(|i| i.selectable) {
            self.filter_state.select(Some(pos));
        }

        self.mode = Mode::FilterPopup;
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header bar
            Constraint::Min(5),    // body (list + detail)
            Constraint::Length(1), // footer / help bar
        ])
        .split(frame.area());

    draw_header(frame, chunks[0], app);
    draw_body(frame, chunks[1], app);
    draw_footer(frame, chunks[2], app);

    if matches!(app.mode, Mode::FilterPopup) {
        draw_filter_popup(frame, app);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!(" 読 Toku ── {} ({}) ", app.active_filter, app.books.len());
    let bar = Paragraph::new(title).style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(bar, area);
}

fn draw_body(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    draw_book_list(frame, chunks[0], app);
    draw_book_detail(frame, chunks[1], app);
}

fn draw_book_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Library ");

    if app.books.is_empty() {
        let msg = Paragraph::new(
            "\n  No books found.\n\n  Import with:\n    toku import goodreads <file>\n\n  Or add manually:\n    toku add --isbn <isbn>",
        )
        .style(Style::default().fg(Color::DarkGray))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let max_title = area.width.saturating_sub(7) as usize; // borders + icon + padding

    let items: Vec<ListItem> = app
        .books
        .iter()
        .map(|book| {
            let (icon, icon_color) = match book.status {
                ReadingStatus::Read => ("✓", Color::Green),
                ReadingStatus::Reading => ("◉", Color::Yellow),
                ReadingStatus::WantToRead => ("○", Color::DarkGray),
                ReadingStatus::Abandoned => ("✗", Color::Red),
                ReadingStatus::OnHold => ("⏸", Color::Magenta),
            };

            let title = truncate_str(&book.title, max_title);

            ListItem::new(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
                Span::raw(title),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_book_detail(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Details ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let idx = match app.list_state.selected() {
        Some(i) if i < app.books.len() => i,
        _ => {
            let msg = Paragraph::new("  Select a book to view details")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, inner);
            return;
        }
    };

    let book = &app.books[idx];
    let detail = match &app.detail {
        Some(d) => d,
        None => return,
    };

    let content_width = inner.width.saturating_sub(2) as usize;
    let label_w = 10;

    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(vec![
        Span::styled("Title     ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            book.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));

    if let Some(ref sub) = book.subtitle {
        lines.push(Line::from(vec![
            Span::raw("          "),
            Span::styled(
                sub.clone(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    // Author
    let author_text = if detail.authors.is_empty() {
        "—".to_string()
    } else {
        detail.authors.join(", ")
    };
    lines.push(Line::from(vec![
        Span::styled("Author    ", Style::default().fg(Color::DarkGray)),
        Span::raw(author_text),
    ]));

    lines.push(Line::from(""));

    // Status
    let (status_label, status_color) = match book.status {
        ReadingStatus::Read => ("Read", Color::Green),
        ReadingStatus::Reading => ("Reading", Color::Yellow),
        ReadingStatus::WantToRead => ("Want to Read", Color::Blue),
        ReadingStatus::Abandoned => ("DNF", Color::Red),
        ReadingStatus::OnHold => ("On Hold", Color::Magenta),
    };
    lines.push(Line::from(vec![
        Span::styled("Status    ", Style::default().fg(Color::DarkGray)),
        Span::styled(status_label, Style::default().fg(status_color)),
    ]));

    // Format + pages/duration
    let format_label = match book.format {
        BookFormat::Physical => "Print",
        BookFormat::Ebook => "Ebook",
        BookFormat::Audiobook => "Audio",
    };
    let format_detail = match (book.page_count, book.duration_minutes) {
        (Some(p), _) => format!("{format_label} · {p} pages"),
        (_, Some(d)) => format!("{format_label} · {}h {}m", d / 60, d % 60),
        _ => format_label.to_string(),
    };
    lines.push(Line::from(vec![
        Span::styled("Format    ", Style::default().fg(Color::DarkGray)),
        Span::raw(format_detail),
    ]));

    // Rating
    if let Some(rating) = book.rating {
        let stars = rating as f32 / 2.0;
        let full = stars as usize;
        let half = (stars - full as f32) >= 0.4;
        let mut star_str = "★".repeat(full);
        if half {
            star_str.push('½');
        }
        let filled = full + usize::from(half);
        star_str.push_str(&"☆".repeat(5usize.saturating_sub(filled)));

        lines.push(Line::from(vec![
            Span::styled("Rating    ", Style::default().fg(Color::DarkGray)),
            Span::styled(star_str, Style::default().fg(Color::Yellow)),
            Span::raw(format!(" ({:.1})", stars)),
        ]));
    }

    // Pub date
    if let Some(ref date) = book.pub_date {
        lines.push(Line::from(vec![
            Span::styled("Published ", Style::default().fg(Color::DarkGray)),
            Span::raw(date.clone()),
        ]));
    }

    // Language
    if let Some(ref lang) = book.language {
        lines.push(Line::from(vec![
            Span::styled("Language  ", Style::default().fg(Color::DarkGray)),
            Span::raw(lang.clone()),
        ]));
    }

    // ISBNs
    if !detail.isbns.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("ISBN      ", Style::default().fg(Color::DarkGray)),
            Span::raw(detail.isbns.join(", ")),
        ]));
    }

    // Tags
    if !detail.tags.is_empty() {
        lines.push(Line::from(""));
        let tag_spans: Vec<Span> = detail
            .tags
            .iter()
            .enumerate()
            .flat_map(|(i, t)| {
                let mut spans = Vec::new();
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    format!(" {t} "),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans
            })
            .collect();
        lines.push(Line::from(
            std::iter::once(Span::styled(
                "Tags      ",
                Style::default().fg(Color::DarkGray),
            ))
            .chain(tag_spans)
            .collect::<Vec<_>>(),
        ));
    }

    // Description (word-wrapped by Paragraph)
    if let Some(ref desc) = book.description {
        lines.push(Line::from(""));
        // Truncate extremely long descriptions to keep the TUI responsive
        let max_chars = content_width.saturating_add(label_w) * 15;
        let display_desc = truncate_str(desc, max_chars);
        lines.push(Line::from(Span::styled(
            display_desc,
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let help = match app.mode {
        Mode::Normal => " ↑↓/jk Navigate │ PgUp/PgDn Page │ f Filter │ Home/End Jump │ q Quit ",
        Mode::FilterPopup => " ↑↓ Select │ Enter Apply │ Esc Cancel ",
    };
    let bar = Paragraph::new(help)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(bar, area);
}

fn draw_filter_popup(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let popup_w = 36u16.min(area.width.saturating_sub(4));
    let popup_h = (app.filter_items.len() as u16 + 2).min(area.height.saturating_sub(4));
    let popup = Rect::new(
        area.width.saturating_sub(popup_w) / 2,
        area.height.saturating_sub(popup_h) / 2,
        popup_w,
        popup_h,
    );

    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = app
        .filter_items
        .iter()
        .map(|fi| {
            let style = if fi.selectable {
                Style::default()
            } else {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            };
            ListItem::new(fi.label.as_str()).style(style)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Filter ")
        .title_alignment(Alignment::Center)
        .style(Style::default().bg(Color::Black));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, popup, &mut app.filter_state);
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Launch the interactive TUI browser.
pub fn run(repo: &BookRepository) -> Result<()> {
    if !io::stdout().is_terminal() {
        anyhow::bail!(
            "TUI requires an interactive terminal. Use 'toku list' for non-interactive output."
        );
    }

    terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)?;

    // RAII guard: even if we panic, the terminal is restored
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(repo)?;

    while app.running {
        terminal.draw(|frame| draw(frame, &mut app))?;

        if event::poll(std::time::Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key);
        }
    }

    Ok(())
}
