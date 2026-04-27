use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Book, BookFormat, ReadingProgress, ReadingSession, ReadingStatus};

/// Aggregated reading statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingStats {
    pub total_books: usize,
    pub books_read: usize,
    pub books_reading: usize,
    pub books_want_to_read: usize,
    pub books_abandoned: usize,
    pub total_pages_read: i64,
    /// Average rating on the 0–10 scale, or `None` if no books are rated.
    pub average_rating: Option<f64>,
    /// Average rating on a 0–5 star scale, or `None` if no books are rated.
    pub average_rating_stars: Option<f64>,
    /// Books finished per month in the selected period.
    pub books_per_month: f64,
    /// Pages read per day in the selected period.
    pub pages_per_day: f64,
    pub format_breakdown: FormatBreakdown,
    pub currently_reading: Vec<CurrentlyReading>,
}

/// Count of books per physical format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatBreakdown {
    pub physical: usize,
    pub ebook: usize,
    pub audiobook: usize,
}

/// A book the user is currently reading, with progress info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentlyReading {
    pub title: String,
    pub author: String,
    pub latest_page: Option<i32>,
    pub total_pages: Option<i32>,
    pub percent: Option<f64>,
}

/// Input data for a currently-reading book gathered by the DB layer.
pub struct CurrentlyReadingInput {
    pub title: String,
    pub author: String,
    pub page_count: Option<i32>,
    pub latest_progress: Option<ReadingProgress>,
}

/// Compute reading statistics from pure data — no database access.
///
/// * `books` — all books in the library (or the filtered subset).
/// * `sessions` — reading sessions in the selected period.
/// * `currently_reading` — details about books with status `Reading`.
/// * `now` — the current timestamp (injected for testability).
pub fn compute_stats(
    books: &[Book],
    sessions: &[ReadingSession],
    currently_reading: &[CurrentlyReadingInput],
    now: DateTime<Utc>,
) -> ReadingStats {
    let total_books = books.len();

    let books_read = books
        .iter()
        .filter(|b| b.status == ReadingStatus::Read)
        .count();
    let books_reading = books
        .iter()
        .filter(|b| b.status == ReadingStatus::Reading)
        .count();
    let books_want_to_read = books
        .iter()
        .filter(|b| b.status == ReadingStatus::WantToRead)
        .count();
    let books_abandoned = books
        .iter()
        .filter(|b| b.status == ReadingStatus::Abandoned)
        .count();

    // Total pages read: sum page_count of all "Read" books that have one.
    let total_pages_read: i64 = books
        .iter()
        .filter(|b| b.status == ReadingStatus::Read)
        .filter_map(|b| b.page_count.map(|p| p as i64))
        .sum();

    // Average rating (0–10 scale)
    let rated: Vec<i32> = books.iter().filter_map(|b| b.rating).collect();
    let average_rating = if rated.is_empty() {
        None
    } else {
        Some(rated.iter().map(|&r| r as f64).sum::<f64>() / rated.len() as f64)
    };
    let average_rating_stars = average_rating.map(|r| r / 2.0);

    // Reading pace: based on finished sessions in the period.
    let finished_sessions: Vec<&ReadingSession> = sessions
        .iter()
        .filter(|s| s.finished_at.is_some())
        .collect();
    let (books_per_month, pages_per_day) = compute_pace(&finished_sessions, books, now);

    // Format breakdown
    let format_breakdown = FormatBreakdown {
        physical: books
            .iter()
            .filter(|b| b.format == BookFormat::Physical)
            .count(),
        ebook: books
            .iter()
            .filter(|b| b.format == BookFormat::Ebook)
            .count(),
        audiobook: books
            .iter()
            .filter(|b| b.format == BookFormat::Audiobook)
            .count(),
    };

    // Currently reading details
    let currently_reading_list: Vec<CurrentlyReading> = currently_reading
        .iter()
        .map(|cr| {
            let latest_page = cr.latest_progress.as_ref().map(|p| p.value);

            let percent = match (latest_page, cr.page_count) {
                (Some(page), Some(total)) if total > 0 => {
                    Some((page as f64 / total as f64 * 100.0).min(100.0))
                }
                _ => None,
            };

            CurrentlyReading {
                title: cr.title.clone(),
                author: cr.author.clone(),
                latest_page,
                total_pages: cr.page_count,
                percent,
            }
        })
        .collect();

    ReadingStats {
        total_books,
        books_read,
        books_reading,
        books_want_to_read,
        books_abandoned,
        total_pages_read,
        average_rating,
        average_rating_stars,
        books_per_month,
        pages_per_day,
        format_breakdown,
        currently_reading: currently_reading_list,
    }
}

/// Compute books/month and pages/day from finished sessions.
fn compute_pace(
    finished_sessions: &[&ReadingSession],
    books: &[Book],
    now: DateTime<Utc>,
) -> (f64, f64) {
    if finished_sessions.is_empty() {
        return (0.0, 0.0);
    }

    // Find the date range spanned by finished sessions.
    let earliest = finished_sessions.iter().filter_map(|s| s.finished_at).min();
    let latest = finished_sessions.iter().filter_map(|s| s.finished_at).max();

    let (span_months, span_days) = match (earliest, latest) {
        (Some(first), Some(_last)) => {
            let duration = now.signed_duration_since(first);
            let days = duration.num_days().max(1) as f64;
            let months = (days / 30.44).max(1.0); // average days/month
            (months, days)
        }
        _ => return (0.0, 0.0),
    };

    let finished_count = finished_sessions.len() as f64;
    let books_per_month = finished_count / span_months;

    // Pages/day: sum pages of books finished in the period.
    let finished_book_ids: Vec<_> = finished_sessions.iter().map(|s| s.book_id).collect();
    let pages: i64 = books
        .iter()
        .filter(|b| finished_book_ids.contains(&b.id))
        .filter_map(|b| b.page_count.map(|p| p as i64))
        .sum();
    let pages_per_day = pages as f64 / span_days;

    (books_per_month, pages_per_day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn make_book(title: &str, status: ReadingStatus, format: BookFormat) -> Book {
        let mut book = Book::new(title);
        book.status = status;
        book.format = format;
        book
    }

    fn make_session(
        book_id: Uuid,
        started: DateTime<Utc>,
        finished: Option<DateTime<Utc>>,
    ) -> ReadingSession {
        let mut session = ReadingSession::new(book_id);
        session.started_at = started;
        session.finished_at = finished;
        session
    }

    #[test]
    fn empty_library_returns_zeroed_stats() {
        let now = Utc::now();
        let stats = compute_stats(&[], &[], &[], now);

        assert_eq!(stats.total_books, 0);
        assert_eq!(stats.books_read, 0);
        assert_eq!(stats.books_reading, 0);
        assert_eq!(stats.books_want_to_read, 0);
        assert_eq!(stats.books_abandoned, 0);
        assert_eq!(stats.total_pages_read, 0);
        assert!(stats.average_rating.is_none());
        assert!(stats.average_rating_stars.is_none());
        assert_eq!(stats.books_per_month, 0.0);
        assert_eq!(stats.pages_per_day, 0.0);
        assert_eq!(stats.format_breakdown.physical, 0);
        assert_eq!(stats.format_breakdown.ebook, 0);
        assert_eq!(stats.format_breakdown.audiobook, 0);
        assert!(stats.currently_reading.is_empty());
    }

    #[test]
    fn average_rating_handles_no_rated_books() {
        let mut book1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        book1.rating = None;
        let book2 = make_book("B", ReadingStatus::WantToRead, BookFormat::Ebook);

        let stats = compute_stats(&[book1, book2], &[], &[], Utc::now());
        assert!(stats.average_rating.is_none());
        assert!(stats.average_rating_stars.is_none());
    }

    #[test]
    fn average_rating_computation() {
        let mut book1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        book1.rating = Some(8); // 4.0★
        let mut book2 = make_book("B", ReadingStatus::Read, BookFormat::Ebook);
        book2.rating = Some(6); // 3.0★
        let book3 = make_book("C", ReadingStatus::WantToRead, BookFormat::Physical);

        let stats = compute_stats(&[book1, book2, book3], &[], &[], Utc::now());
        let avg = stats.average_rating.expect("should have an average");
        assert!((avg - 7.0).abs() < f64::EPSILON); // (8+6)/2 = 7.0
        let stars = stats.average_rating_stars.expect("should have stars");
        assert!((stars - 3.5).abs() < f64::EPSILON); // 7.0 / 2 = 3.5
    }

    #[test]
    fn format_breakdown_counts() {
        let books = vec![
            make_book("A", ReadingStatus::Read, BookFormat::Physical),
            make_book("B", ReadingStatus::Read, BookFormat::Physical),
            make_book("C", ReadingStatus::Reading, BookFormat::Ebook),
            make_book("D", ReadingStatus::WantToRead, BookFormat::Audiobook),
            make_book("E", ReadingStatus::Read, BookFormat::Ebook),
        ];

        let stats = compute_stats(&books, &[], &[], Utc::now());
        assert_eq!(stats.format_breakdown.physical, 2);
        assert_eq!(stats.format_breakdown.ebook, 2);
        assert_eq!(stats.format_breakdown.audiobook, 1);
    }

    #[test]
    fn status_counts() {
        let books = vec![
            make_book("A", ReadingStatus::Read, BookFormat::Physical),
            make_book("B", ReadingStatus::Read, BookFormat::Ebook),
            make_book("C", ReadingStatus::Reading, BookFormat::Physical),
            make_book("D", ReadingStatus::WantToRead, BookFormat::Physical),
            make_book("E", ReadingStatus::WantToRead, BookFormat::Ebook),
            make_book("F", ReadingStatus::Abandoned, BookFormat::Audiobook),
        ];

        let stats = compute_stats(&books, &[], &[], Utc::now());
        assert_eq!(stats.total_books, 6);
        assert_eq!(stats.books_read, 2);
        assert_eq!(stats.books_reading, 1);
        assert_eq!(stats.books_want_to_read, 2);
        assert_eq!(stats.books_abandoned, 1);
    }

    #[test]
    fn total_pages_read_sums_read_books() {
        let mut book1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        book1.page_count = Some(300);
        let mut book2 = make_book("B", ReadingStatus::Read, BookFormat::Ebook);
        book2.page_count = Some(200);
        let mut book3 = make_book("C", ReadingStatus::Reading, BookFormat::Physical);
        book3.page_count = Some(500); // not counted — still reading
        let book4 = make_book("D", ReadingStatus::Read, BookFormat::Physical);
        // no page count — skipped

        let stats = compute_stats(&[book1, book2, book3, book4], &[], &[], Utc::now());
        assert_eq!(stats.total_pages_read, 500); // 300 + 200
    }

    #[test]
    fn currently_reading_with_progress() {
        let input = vec![CurrentlyReadingInput {
            title: "Dune".to_string(),
            author: "Frank Herbert".to_string(),
            page_count: Some(544),
            latest_progress: Some(ReadingProgress::new(
                Uuid::now_v7(),
                crate::ProgressType::Page,
                145,
            )),
        }];

        let stats = compute_stats(&[], &[], &input, Utc::now());
        assert_eq!(stats.currently_reading.len(), 1);
        let cr = &stats.currently_reading[0];
        assert_eq!(cr.title, "Dune");
        assert_eq!(cr.author, "Frank Herbert");
        assert_eq!(cr.latest_page, Some(145));
        assert_eq!(cr.total_pages, Some(544));
        let pct = cr.percent.expect("should have percent");
        assert!((pct - 26.654).abs() < 0.1);
    }

    #[test]
    fn reading_pace_computation() {
        let mut book = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        book.page_count = Some(300);

        // Session finished 30 days ago
        let now = Utc::now();
        let thirty_days_ago = now - chrono::Duration::days(30);
        let sixty_days_ago = now - chrono::Duration::days(60);
        let session = make_session(book.id, sixty_days_ago, Some(thirty_days_ago));

        let stats = compute_stats(&[book], &[session], &[], now);

        // 1 book over ~1 month
        assert!(stats.books_per_month > 0.0);
        assert!(stats.pages_per_day > 0.0);
    }
}
