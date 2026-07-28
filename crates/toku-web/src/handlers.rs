//! Axum route handlers for the statistics dashboard.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::response::{Html, Redirect};
use chrono::Datelike;
use toku_core::{CurrentlyReadingInput, StatsInput, compute_stats};
use toku_db::{BookRepository, Database};

use crate::AppState;
use crate::error::WebError;
use crate::views;

/// `GET /` → redirect to `/library`.
pub async fn root() -> Redirect {
    Redirect::permanent("/library")
}

#[derive(serde::Deserialize)]
pub struct StatsQuery {
    pub year: Option<i32>,
}

/// `GET /stats` — main statistics dashboard.
pub async fn stats_dashboard(
    State(state): State<AppState>,
    Query(query): Query<StatsQuery>,
) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();
    let year = query.year;

    let (stats, available_years) =
        tokio::task::spawn_blocking(move || gather_stats(&db_path, year))
            .await
            .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Html(
        views::dashboard(&stats, year, &available_years).into_string(),
    ))
}

/// `GET /stats/wrap/:year` — yearly wrap-up.
pub async fn yearly_wrap(
    State(state): State<AppState>,
    Path(year): Path<i32>,
) -> Result<Html<String>, WebError> {
    let db_path = state.db_path.clone();

    let (stats, _) = tokio::task::spawn_blocking(move || gather_stats(&db_path, Some(year)))
        .await
        .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(Html(views::yearly_wrap(&stats, year).into_string()))
}

/// `GET /api/stats` — JSON statistics endpoint.
pub async fn stats_json(
    State(state): State<AppState>,
    Query(query): Query<StatsQuery>,
) -> Result<axum::Json<toku_core::ReadingStats>, WebError> {
    let db_path = state.db_path.clone();
    let year = query.year;

    let (stats, _) = tokio::task::spawn_blocking(move || gather_stats(&db_path, year))
        .await
        .map_err(|e| WebError::Internal(e.to_string()))??;

    Ok(axum::Json(stats))
}

// ── Data gathering ──────────────────────────────────────────────────

/// Gather stats from the database — runs on a blocking thread.
///
/// Returns `(ReadingStats, available_years)`.
fn gather_stats(
    db_path: &std::path::Path,
    year: Option<i32>,
) -> Result<(toku_core::ReadingStats, Vec<i32>), WebError> {
    let db = Database::open_no_migrate_default(db_path)?;
    let repo = BookRepository::new(&db);

    let books = repo.list_books()?;
    let book_ids: Vec<String> = books.iter().map(|b| b.id.to_string()).collect();

    let sessions = match year {
        Some(y) => repo.list_reading_sessions_in_year(y)?,
        None => repo.list_reading_sessions()?,
    };

    let currently_reading_details = repo.get_currently_reading_details()?;
    let currently_reading_input: Vec<CurrentlyReadingInput> = currently_reading_details
        .into_iter()
        .map(|(book, progress, authors)| {
            let author_name = authors
                .into_iter()
                .map(|(a, _)| a.name)
                .collect::<Vec<_>>()
                .join(", ");
            CurrentlyReadingInput {
                title: book.title,
                author: author_name,
                page_count: book.page_count,
                latest_progress: progress,
            }
        })
        .collect();

    let tag_counts = repo.list_tag_counts()?;
    let author_counts = repo.list_author_book_counts()?;

    let activity_dates = match year {
        Some(y) => repo.list_activity_dates_in_year(y)?,
        None => repo.list_activity_dates()?,
    };

    let now = chrono::Utc::now();
    let today = chrono::Local::now().date_naive();

    let mood_tag_data: HashMap<String, Vec<String>> = repo.get_mood_tags_for_books(&book_ids)?;

    let stats = compute_stats(StatsInput {
        books: &books,
        sessions: &sessions,
        currently_reading: &currently_reading_input,
        tag_counts: &tag_counts,
        author_counts: &author_counts,
        activity_dates: &activity_dates,
        now,
        today,
        mood_tag_data: &mood_tag_data,
    });

    // Derive available years from all sessions
    let all_sessions = repo.list_reading_sessions()?;
    let mut years: Vec<i32> = all_sessions
        .iter()
        .filter_map(|s| s.finished_at.map(|f| f.date_naive().year()))
        .collect();
    years.sort();
    years.dedup();

    Ok((stats, years))
}
