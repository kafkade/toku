//! Axum route handlers for the library browser, book detail, search, and cover serving.

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use toku_core::{
    Author, Book, BookAuthor, ContributorRole, ReadingProgress, ReadingSession, ReadingStatus, Tag,
    TagCount,
};
use toku_db::{BookRepository, Database};

use crate::AppState;
use crate::error::WebError;
use crate::library_views;

// ── Query params ────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct LibraryQuery {
    pub view: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
    pub sort: Option<String>,
    pub page: Option<usize>,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
}

// ── Data bundles ────────────────────────────────────────────────────

/// Lightweight book summary for grid/list views.
pub struct BookCard {
    pub book: Book,
    pub author_display: String,
}

/// Full book detail data bundle.
pub struct BookDetailData {
    pub book: Book,
    pub authors: Vec<(Author, BookAuthor)>,
    pub tags: Vec<Tag>,
    pub sessions: Vec<ReadingSession>,
    pub progress_log: Vec<ReadingProgress>,
    pub latest_progress: Option<ReadingProgress>,
}

const PAGE_SIZE: usize = 60;

// ── Handlers ────────────────────────────────────────────────────────

/// `GET /library` — library grid/list view with filters and pagination.
pub async fn library_page(
    State(state): State<AppState>,
    Query(query): Query<LibraryQuery>,
) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();
    let status = query.status.clone();
    let tag = query.tag.clone();
    let sort = query.sort.clone().unwrap_or_else(|| "title".into());
    let page = query.page.unwrap_or(1).max(1);

    let (cards, total, tag_counts) = tokio::task::spawn_blocking(move || {
        gather_library(&db_path, status.as_deref(), tag.as_deref(), &sort, page)
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    let view = query.view.as_deref().unwrap_or("grid");
    let total_pages = (total + PAGE_SIZE - 1) / PAGE_SIZE.max(1);

    Ok(Html(
        library_views::library_page(
            &cards,
            view,
            query.status.as_deref(),
            query.tag.as_deref(),
            query.sort.as_deref().unwrap_or("title"),
            page,
            total,
            total_pages,
            &tag_counts,
        )
        .into_string(),
    ))
}

/// `GET /books/{id}` — book detail page.
pub async fn book_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();

    let detail = tokio::task::spawn_blocking(move || gather_book_detail(&db_path, &id))
        .await
        .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Html(library_views::book_detail_page(&detail).into_string()))
}

/// `GET /search` — search with FTS5 and filters.
pub async fn search_page(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();
    let q = query.q.clone();
    let status = query.status.clone();
    let tag = query.tag.clone();

    let (results, tag_counts) = tokio::task::spawn_blocking(move || {
        gather_search(&db_path, q.as_deref(), status.as_deref(), tag.as_deref())
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Html(
        library_views::search_page(
            query.q.as_deref(),
            query.status.as_deref(),
            query.tag.as_deref(),
            &results,
            &tag_counts,
        )
        .into_string(),
    ))
}

/// `GET /covers/{hash}` — serve a cover image from disk.
pub async fn serve_cover(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    // Strict validation: hex chars only, max 64 chars
    if hash.is_empty() || hash.len() > 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StatusCode::NOT_FOUND);
    }

    let path = state.covers_dir.join(format!("{hash}.jpg"));

    let data = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok((
        [(header::CONTENT_TYPE, "image/jpeg")],
        [(header::CACHE_CONTROL, "public, max-age=31536000, immutable")],
        data,
    ))
}

// ── Data gathering (runs on blocking thread) ────────────────────────

fn gather_library(
    db_path: &std::path::Path,
    status: Option<&str>,
    tag: Option<&str>,
    sort: &str,
    page: usize,
) -> Result<(Vec<BookCard>, usize, Vec<TagCount>), WebError> {
    let db = Database::open_no_migrate_default(db_path)?;
    let repo = BookRepository::new(&db);

    let mut books = match tag {
        Some(t) => repo.list_books_by_tag(t)?,
        None => repo.list_books()?,
    };

    if let Some(s) = status
        && let Ok(target) = s.parse::<ReadingStatus>()
    {
        books.retain(|b| b.status == target);
    }

    let total = books.len();

    match sort {
        "rating" => books.sort_by_key(|b| std::cmp::Reverse(b.rating)),
        "added" => books.sort_by_key(|b| std::cmp::Reverse(b.created_at)),
        "author" => {
            // Sort by primary author — requires fetching authors
            let mut book_authors: Vec<(Book, String)> = books
                .into_iter()
                .map(|b| {
                    let author = primary_author(&repo, &b);
                    (b, author)
                })
                .collect();
            book_authors.sort_by_key(|a| a.1.to_lowercase());

            let start = (page - 1) * PAGE_SIZE;
            let cards = book_authors
                .into_iter()
                .skip(start)
                .take(PAGE_SIZE)
                .map(|(book, author_display)| BookCard {
                    book,
                    author_display,
                })
                .collect();

            let tag_counts = repo.list_tag_counts()?;
            return Ok((cards, total, tag_counts));
        }
        _ => {} // already sorted by title from repo
    }

    let start = (page - 1) * PAGE_SIZE;
    let page_books: Vec<Book> = books.into_iter().skip(start).take(PAGE_SIZE).collect();

    let cards: Vec<BookCard> = page_books
        .into_iter()
        .map(|book| {
            let author_display = primary_author(&repo, &book);
            BookCard {
                book,
                author_display,
            }
        })
        .collect();

    let tag_counts = repo.list_tag_counts()?;
    Ok((cards, total, tag_counts))
}

fn gather_book_detail(db_path: &std::path::Path, id_str: &str) -> Result<BookDetailData, WebError> {
    let id =
        uuid::Uuid::parse_str(id_str).map_err(|_| WebError::NotFound("invalid book ID".into()))?;

    let db = Database::open_no_migrate_default(db_path)?;
    let repo = BookRepository::new(&db);

    let book = repo
        .get_book(&id)
        .map_err(|_| WebError::NotFound("book not found".into()))?;
    let authors = repo.get_book_authors(&id)?;
    let tags = repo.get_book_tags(&id)?;
    let sessions = repo.list_sessions_for_books(&[id.to_string()])?;
    let progress_log = repo.get_reading_log(&id)?;
    let latest_progress = repo.get_latest_progress(&id)?;

    Ok(BookDetailData {
        book,
        authors,
        tags,
        sessions,
        progress_log,
        latest_progress,
    })
}

fn gather_search(
    db_path: &std::path::Path,
    q: Option<&str>,
    status: Option<&str>,
    tag: Option<&str>,
) -> Result<(Vec<BookCard>, Vec<TagCount>), WebError> {
    let db = Database::open_no_migrate_default(db_path)?;
    let repo = BookRepository::new(&db);

    let books = match q {
        Some(query) if !query.trim().is_empty() => {
            repo.search_books_filtered(query, status, None, tag)?
        }
        _ => Vec::new(),
    };

    let cards: Vec<BookCard> = books
        .into_iter()
        .map(|book| {
            let author_display = primary_author(&repo, &book);
            BookCard {
                book,
                author_display,
            }
        })
        .collect();

    let tag_counts = repo.list_tag_counts()?;
    Ok((cards, tag_counts))
}

/// Get the primary author display string for a book.
fn primary_author(repo: &BookRepository, book: &Book) -> String {
    repo.get_book_authors(&book.id)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, ba)| ba.role == ContributorRole::Author)
        .map(|(a, _)| a.name)
        .collect::<Vec<_>>()
        .join(", ")
}
