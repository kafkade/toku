use std::collections::HashMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::{Book, BookFormat, ReadingProgress, ReadingSession, ReadingStatus};

// ---------------------------------------------------------------------------
// Input types — gathered by the DB layer, consumed by pure computation
// ---------------------------------------------------------------------------

/// All inputs needed to compute reading statistics.
/// Collected by the DB/CLI layer and passed to `compute_stats`.
pub struct StatsInput<'a> {
    pub books: &'a [Book],
    pub sessions: &'a [ReadingSession],
    pub currently_reading: &'a [CurrentlyReadingInput],
    pub tag_counts: &'a [TagCount],
    pub author_counts: &'a [AuthorCount],
    /// Unique dates of reading activity (progress logs, session dates),
    /// sorted ascending. Should be in the user's local timezone.
    pub activity_dates: &'a [NaiveDate],
    /// The current timestamp (injected for testability).
    pub now: DateTime<Utc>,
    /// Today's date in the user's local timezone (for streak calculation).
    pub today: NaiveDate,
    /// Mood tags per book (book_id → list of mood tag names).
    /// Used for `--mood-trends`. Pass an empty map to skip.
    pub mood_tag_data: &'a HashMap<String, Vec<String>>,
}

/// Input data for a currently-reading book gathered by the DB layer.
pub struct CurrentlyReadingInput {
    pub title: String,
    pub author: String,
    pub page_count: Option<i32>,
    pub latest_progress: Option<ReadingProgress>,
}

/// Tag name with book count, provided by the DB layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCount {
    pub name: String,
    pub count: usize,
}

/// Author name with book count, provided by the DB layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorCount {
    pub name: String,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Output types — the computed statistics
// ---------------------------------------------------------------------------

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
    pub rating_distribution: RatingDistribution,
    pub tag_distribution: Vec<TagCount>,
    pub author_stats: AuthorStats,
    pub reading_streaks: ReadingStreaks,
    /// Average days to finish a book, or `None` if no books have been finished.
    pub avg_days_to_finish: Option<f64>,
    /// Weighted reading speed in pages per hour, or `None` if no session data.
    pub reading_speed_pages_per_hour: Option<f64>,
    /// Books finished per month. When scoped to a year, contains all 12 months.
    pub monthly_finished: Vec<MonthlyFinished>,
    pub shortest_book: Option<BookStat>,
    pub longest_book: Option<BookStat>,
    /// Mood tag distribution per month (populated only with `--mood-trends`).
    pub mood_trends: Vec<MoodTrend>,
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

/// Rating distribution: count of books at each rating level 0–10.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatingDistribution {
    /// `counts[i]` = number of books with rating `i` (index 0–10).
    pub counts: [usize; 11],
    pub total_rated: usize,
}

/// Author diversity and top-author statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorStats {
    pub unique_count: usize,
    /// Top authors by book count (up to 10).
    pub top_authors: Vec<AuthorCount>,
}

/// Reading streak information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingStreaks {
    /// Current consecutive days with reading activity (0 if no activity today/yesterday).
    pub current_streak_days: u32,
    /// Longest consecutive streak ever recorded.
    pub longest_streak_days: u32,
    /// Total distinct days with reading activity.
    pub total_active_days: u32,
}

/// Books finished in a calendar month.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthlyFinished {
    pub year: i32,
    pub month: u32,
    pub count: usize,
}

/// A book with its page count, used for shortest/longest stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookStat {
    pub title: String,
    pub page_count: i32,
}

/// Mood tag distribution for a single month.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoodTrend {
    pub year: i32,
    pub month: u32,
    pub moods: Vec<TagCount>,
}

// ---------------------------------------------------------------------------
// Computation
// ---------------------------------------------------------------------------

/// Compute reading statistics from pure data — no database access.
pub fn compute_stats(input: StatsInput<'_>) -> ReadingStats {
    let books = input.books;
    let sessions = input.sessions;
    let now = input.now;

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
    let rated: Vec<i32> = books
        .iter()
        .filter_map(|b| b.rating)
        .filter(|&r| (0..=10).contains(&r))
        .collect();
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
    let currently_reading = compute_currently_reading(input.currently_reading);

    // Rating distribution
    let rating_distribution = compute_rating_distribution(books);

    // Tag distribution (already sorted by DB, take top 20)
    let tag_distribution: Vec<TagCount> = input.tag_counts.iter().take(20).cloned().collect();

    // Author stats
    let author_stats = AuthorStats {
        unique_count: input.author_counts.len(),
        top_authors: input.author_counts.iter().take(10).cloned().collect(),
    };

    // Reading streaks
    let reading_streaks = compute_streaks(input.activity_dates, input.today);

    // Time to finish (average days per finished session)
    let avg_days_to_finish = compute_avg_days_to_finish(&finished_sessions);

    // Reading speed (weighted pages/hour)
    let reading_speed_pages_per_hour = compute_reading_speed(sessions);

    // Monthly finished books
    let monthly_finished = compute_monthly_finished(&finished_sessions);

    // Shortest / longest book (all books in scope with page_count > 0)
    let books_with_pages: Vec<&Book> = books
        .iter()
        .filter(|b| b.page_count.is_some_and(|p| p > 0))
        .collect();

    let shortest_book = books_with_pages
        .iter()
        .min_by_key(|b| b.page_count.unwrap())
        .map(|b| BookStat {
            title: b.title.clone(),
            page_count: b.page_count.unwrap(),
        });

    let longest_book = books_with_pages
        .iter()
        .max_by_key(|b| b.page_count.unwrap())
        .map(|b| BookStat {
            title: b.title.clone(),
            page_count: b.page_count.unwrap(),
        });

    // Mood trends: aggregate mood tags of finished books per month
    let mood_trends = compute_mood_trends(&finished_sessions, input.mood_tag_data);

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
        currently_reading,
        rating_distribution,
        tag_distribution,
        author_stats,
        reading_streaks,
        avg_days_to_finish,
        reading_speed_pages_per_hour,
        monthly_finished,
        shortest_book,
        longest_book,
        mood_trends,
    }
}

fn compute_currently_reading(inputs: &[CurrentlyReadingInput]) -> Vec<CurrentlyReading> {
    inputs
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
        .collect()
}

fn compute_rating_distribution(books: &[Book]) -> RatingDistribution {
    let mut counts = [0usize; 11];
    let mut total_rated = 0;
    for book in books {
        if let Some(r) = book.rating
            && (0..=10).contains(&r)
        {
            counts[r as usize] += 1;
            total_rated += 1;
        }
    }
    RatingDistribution {
        counts,
        total_rated,
    }
}

/// Compute current and longest reading streaks from sorted activity dates.
fn compute_streaks(activity_dates: &[NaiveDate], today: NaiveDate) -> ReadingStreaks {
    if activity_dates.is_empty() {
        return ReadingStreaks {
            current_streak_days: 0,
            longest_streak_days: 0,
            total_active_days: 0,
        };
    }

    // Deduplicate and sort (should already be sorted, but ensure)
    let mut dates: Vec<NaiveDate> = activity_dates.to_vec();
    dates.sort();
    dates.dedup();

    let total_active_days = dates.len() as u32;

    let mut longest = 1u32;
    let mut current = 1u32;

    for window in dates.windows(2) {
        let diff = window[1].signed_duration_since(window[0]).num_days();
        if diff == 1 {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 1;
        }
    }

    // Determine current streak: the streak must include today or yesterday.
    let last_date = *dates.last().unwrap();
    let gap = today.signed_duration_since(last_date).num_days();
    let current_streak = if gap <= 1 { current } else { 0 };

    ReadingStreaks {
        current_streak_days: current_streak,
        longest_streak_days: longest,
        total_active_days,
    }
}

/// Average days to finish a book, based on completed sessions.
fn compute_avg_days_to_finish(finished_sessions: &[&ReadingSession]) -> Option<f64> {
    let durations: Vec<f64> = finished_sessions
        .iter()
        .filter_map(|s| {
            s.finished_at.map(|fin| {
                let days = fin.signed_duration_since(s.started_at).num_hours() as f64 / 24.0;
                days.max(0.0)
            })
        })
        .collect();

    if durations.is_empty() {
        None
    } else {
        Some(durations.iter().sum::<f64>() / durations.len() as f64)
    }
}

/// Weighted reading speed: total pages / total hours across all valid sessions.
/// Only includes sessions with valid page range and non-zero duration.
fn compute_reading_speed(sessions: &[ReadingSession]) -> Option<f64> {
    let mut total_pages: f64 = 0.0;
    let mut total_hours: f64 = 0.0;

    for s in sessions {
        let (start_p, end_p, finished) = match (s.start_page, s.end_page, s.finished_at) {
            (Some(sp), Some(ep), Some(fin)) if ep > sp => (sp, ep, fin),
            _ => continue,
        };

        let hours = finished.signed_duration_since(s.started_at).num_minutes() as f64 / 60.0;
        if hours > 0.0 {
            total_pages += (end_p - start_p) as f64;
            total_hours += hours;
        }
    }

    if total_hours > 0.0 {
        Some(total_pages / total_hours)
    } else {
        None
    }
}

/// Books finished per month from completed sessions.
fn compute_monthly_finished(finished_sessions: &[&ReadingSession]) -> Vec<MonthlyFinished> {
    let mut counts: HashMap<(i32, u32), usize> = HashMap::new();

    for s in finished_sessions {
        if let Some(fin) = s.finished_at {
            let key = (fin.year(), fin.month());
            *counts.entry(key).or_default() += 1;
        }
    }

    // If we have data, fill in all months between min and max.
    if counts.is_empty() {
        return Vec::new();
    }

    let min_key = *counts.keys().min().unwrap();
    let max_key = *counts.keys().max().unwrap();

    let mut result = Vec::new();
    let mut year = min_key.0;
    let mut month = min_key.1;

    loop {
        let count = counts.get(&(year, month)).copied().unwrap_or(0);
        result.push(MonthlyFinished { year, month, count });

        if (year, month) == max_key {
            break;
        }

        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }

    result
}

/// Compute mood tag distribution per month from finished sessions.
fn compute_mood_trends(
    finished_sessions: &[&ReadingSession],
    mood_tag_data: &HashMap<String, Vec<String>>,
) -> Vec<MoodTrend> {
    if mood_tag_data.is_empty() {
        return Vec::new();
    }

    // Group finished book_ids by month
    let mut month_moods: HashMap<(i32, u32), HashMap<String, usize>> = HashMap::new();

    for s in finished_sessions {
        if let Some(fin) = s.finished_at {
            let key = (fin.year(), fin.month());
            let book_id = s.book_id.to_string();
            if let Some(moods) = mood_tag_data.get(&book_id) {
                let month_entry = month_moods.entry(key).or_default();
                for mood in moods {
                    *month_entry.entry(mood.clone()).or_default() += 1;
                }
            }
        }
    }

    if month_moods.is_empty() {
        return Vec::new();
    }

    let mut result: Vec<MoodTrend> = month_moods
        .into_iter()
        .map(|((year, month), counts)| {
            let mut moods: Vec<TagCount> = counts
                .into_iter()
                .map(|(name, count)| TagCount { name, count })
                .collect();
            moods.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
            MoodTrend { year, month, moods }
        })
        .collect();

    result.sort_by_key(|t| (t.year, t.month));
    result
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

    fn empty_input(now: DateTime<Utc>) -> StatsInput<'static> {
        static EMPTY_MOODS: std::sync::LazyLock<HashMap<String, Vec<String>>> =
            std::sync::LazyLock::new(HashMap::new);
        StatsInput {
            books: &[],
            sessions: &[],
            currently_reading: &[],
            tag_counts: &[],
            author_counts: &[],
            activity_dates: &[],
            now,
            today: now.date_naive(),
            mood_tag_data: &EMPTY_MOODS,
        }
    }

    #[test]
    fn empty_library_returns_zeroed_stats() {
        let now = Utc::now();
        let stats = compute_stats(empty_input(now));

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
        assert_eq!(stats.rating_distribution.total_rated, 0);
        assert!(stats.tag_distribution.is_empty());
        assert_eq!(stats.author_stats.unique_count, 0);
        assert_eq!(stats.reading_streaks.current_streak_days, 0);
        assert_eq!(stats.reading_streaks.longest_streak_days, 0);
        assert!(stats.avg_days_to_finish.is_none());
        assert!(stats.reading_speed_pages_per_hour.is_none());
        assert!(stats.monthly_finished.is_empty());
        assert!(stats.shortest_book.is_none());
        assert!(stats.longest_book.is_none());
    }

    #[test]
    fn average_rating_handles_no_rated_books() {
        let mut book1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        book1.rating = None;
        let book2 = make_book("B", ReadingStatus::WantToRead, BookFormat::Ebook);
        let books = [book1, book2];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });
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
        let books = [book1, book2, book3];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });
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

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });
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

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });
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
        let books = [book1, book2, book3, book4];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });
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

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            currently_reading: &input,
            ..empty_input(now)
        });
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
        let books = [book];
        let sessions = [session];

        let stats = compute_stats(StatsInput {
            books: &books,
            sessions: &sessions,
            ..empty_input(now)
        });

        // 1 book over ~1 month
        assert!(stats.books_per_month > 0.0);
        assert!(stats.pages_per_day > 0.0);
    }

    // --- New statistics tests ---

    #[test]
    fn rating_distribution_buckets() {
        let mut b1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        b1.rating = Some(8);
        let mut b2 = make_book("B", ReadingStatus::Read, BookFormat::Physical);
        b2.rating = Some(8);
        let mut b3 = make_book("C", ReadingStatus::Read, BookFormat::Physical);
        b3.rating = Some(5);
        let mut b4 = make_book("D", ReadingStatus::Read, BookFormat::Physical);
        b4.rating = None; // unrated
        let books = [b1, b2, b3, b4];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });

        assert_eq!(stats.rating_distribution.total_rated, 3);
        assert_eq!(stats.rating_distribution.counts[8], 2);
        assert_eq!(stats.rating_distribution.counts[5], 1);
        assert_eq!(stats.rating_distribution.counts[0], 0);
    }

    #[test]
    fn rating_distribution_ignores_out_of_range() {
        let mut b1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        b1.rating = Some(11); // out of range, should be ignored
        let mut b2 = make_book("B", ReadingStatus::Read, BookFormat::Physical);
        b2.rating = Some(-1); // out of range
        let books = [b1, b2];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });
        assert_eq!(stats.rating_distribution.total_rated, 0);
    }

    #[test]
    fn tag_distribution_passes_through() {
        let tags = vec![
            TagCount {
                name: "sci-fi".to_string(),
                count: 10,
            },
            TagCount {
                name: "fantasy".to_string(),
                count: 5,
            },
        ];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            tag_counts: &tags,
            ..empty_input(now)
        });
        assert_eq!(stats.tag_distribution.len(), 2);
        assert_eq!(stats.tag_distribution[0].name, "sci-fi");
        assert_eq!(stats.tag_distribution[0].count, 10);
    }

    #[test]
    fn tag_distribution_caps_at_20() {
        let tags: Vec<TagCount> = (0..30)
            .map(|i| TagCount {
                name: format!("tag-{i}"),
                count: 30 - i,
            })
            .collect();

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            tag_counts: &tags,
            ..empty_input(now)
        });
        assert_eq!(stats.tag_distribution.len(), 20);
    }

    #[test]
    fn author_stats_from_input() {
        let authors = vec![
            AuthorCount {
                name: "Frank Herbert".to_string(),
                count: 5,
            },
            AuthorCount {
                name: "Ursula K. Le Guin".to_string(),
                count: 3,
            },
        ];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            author_counts: &authors,
            ..empty_input(now)
        });
        assert_eq!(stats.author_stats.unique_count, 2);
        assert_eq!(stats.author_stats.top_authors.len(), 2);
        assert_eq!(stats.author_stats.top_authors[0].name, "Frank Herbert");
    }

    #[test]
    fn streaks_consecutive_days() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let dates = vec![
            NaiveDate::from_ymd_opt(2024, 6, 10).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 11).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(),
            // gap on 13th
            NaiveDate::from_ymd_opt(2024, 6, 14).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        ];

        let streaks = compute_streaks(&dates, today);
        assert_eq!(streaks.current_streak_days, 2); // 14th, 15th
        assert_eq!(streaks.longest_streak_days, 3); // 10th, 11th, 12th
        assert_eq!(streaks.total_active_days, 5);
    }

    #[test]
    fn streaks_no_activity() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let streaks = compute_streaks(&[], today);
        assert_eq!(streaks.current_streak_days, 0);
        assert_eq!(streaks.longest_streak_days, 0);
        assert_eq!(streaks.total_active_days, 0);
    }

    #[test]
    fn streaks_yesterday_counts_as_current() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let dates = vec![
            NaiveDate::from_ymd_opt(2024, 6, 13).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 14).unwrap(),
        ];

        let streaks = compute_streaks(&dates, today);
        assert_eq!(streaks.current_streak_days, 2); // yesterday still counts
    }

    #[test]
    fn streaks_old_activity_not_current() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let dates = vec![
            NaiveDate::from_ymd_opt(2024, 6, 10).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 11).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(),
        ];

        let streaks = compute_streaks(&dates, today);
        assert_eq!(streaks.current_streak_days, 0); // gap of 3 days
        assert_eq!(streaks.longest_streak_days, 3);
    }

    #[test]
    fn avg_days_to_finish_computation() {
        let now = Utc::now();
        let s1 = make_session(
            Uuid::now_v7(),
            now - chrono::Duration::days(10),
            Some(now - chrono::Duration::days(5)),
        );
        let s2 = make_session(
            Uuid::now_v7(),
            now - chrono::Duration::days(20),
            Some(now - chrono::Duration::days(10)),
        );
        let refs: Vec<&ReadingSession> = vec![&s1, &s2];

        let avg = compute_avg_days_to_finish(&refs).expect("should have avg");
        // s1: 5 days, s2: 10 days => avg = 7.5
        assert!((avg - 7.5).abs() < 0.1);
    }

    #[test]
    fn reading_speed_weighted() {
        let now = Utc::now();

        let mut s1 = make_session(Uuid::now_v7(), now - chrono::Duration::hours(2), Some(now));
        s1.start_page = Some(0);
        s1.end_page = Some(60); // 60 pages in 2 hours = 30 pages/hour

        let mut s2 = make_session(
            Uuid::now_v7(),
            now - chrono::Duration::hours(4),
            Some(now - chrono::Duration::hours(2)),
        );
        s2.start_page = Some(0);
        s2.end_page = Some(80); // 80 pages in 2 hours = 40 pages/hour

        let sessions = [s1, s2];
        // Weighted: 140 pages / 4 hours = 35 pages/hour
        let speed = compute_reading_speed(&sessions).expect("should have speed");
        assert!((speed - 35.0).abs() < 0.1);
    }

    #[test]
    fn reading_speed_skips_invalid_sessions() {
        let now = Utc::now();

        // Session with no pages
        let s1 = make_session(Uuid::now_v7(), now - chrono::Duration::hours(2), Some(now));

        // Session not finished
        let mut s2 = make_session(Uuid::now_v7(), now - chrono::Duration::hours(2), None);
        s2.start_page = Some(0);
        s2.end_page = Some(60);

        let sessions = [s1, s2];
        assert!(compute_reading_speed(&sessions).is_none());
    }

    #[test]
    fn monthly_finished_fills_gaps() {
        let base = Utc::now();
        let mut s1 = ReadingSession::new(Uuid::now_v7());
        s1.started_at = base - chrono::Duration::days(90);
        s1.finished_at = Some(
            chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc(),
        );

        let mut s2 = ReadingSession::new(Uuid::now_v7());
        s2.started_at = base - chrono::Duration::days(60);
        s2.finished_at = Some(
            chrono::NaiveDate::from_ymd_opt(2024, 3, 10)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc(),
        );

        let refs: Vec<&ReadingSession> = vec![&s1, &s2];
        let monthly = compute_monthly_finished(&refs);

        assert_eq!(monthly.len(), 3); // Jan, Feb (zero-filled), Mar
        assert_eq!(monthly[0].year, 2024);
        assert_eq!(monthly[0].month, 1);
        assert_eq!(monthly[0].count, 1);
        assert_eq!(monthly[1].month, 2);
        assert_eq!(monthly[1].count, 0); // gap month
        assert_eq!(monthly[2].month, 3);
        assert_eq!(monthly[2].count, 1);
    }

    #[test]
    fn shortest_longest_book() {
        let mut b1 = make_book("Short", ReadingStatus::Read, BookFormat::Physical);
        b1.page_count = Some(100);
        let mut b2 = make_book("Long", ReadingStatus::Read, BookFormat::Physical);
        b2.page_count = Some(900);
        let mut b3 = make_book("No pages", ReadingStatus::Read, BookFormat::Audiobook);
        b3.page_count = None; // excluded
        let books = [b1, b2, b3];

        let now = Utc::now();
        let stats = compute_stats(StatsInput {
            books: &books,
            ..empty_input(now)
        });

        let shortest = stats.shortest_book.expect("should have shortest");
        assert_eq!(shortest.title, "Short");
        assert_eq!(shortest.page_count, 100);

        let longest = stats.longest_book.expect("should have longest");
        assert_eq!(longest.title, "Long");
        assert_eq!(longest.page_count, 900);
    }

    #[test]
    fn mood_trends_empty_when_no_data() {
        let now = Utc::now();
        let stats = compute_stats(empty_input(now));
        assert!(stats.mood_trends.is_empty());
    }

    #[test]
    fn mood_trends_aggregates_by_month() {
        let book1 = make_book("A", ReadingStatus::Read, BookFormat::Physical);
        let book2 = make_book("B", ReadingStatus::Read, BookFormat::Ebook);
        let books = [book1.clone(), book2.clone()];

        let jan = "2024-01-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let feb = "2024-02-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();

        let s1 = make_session(book1.id, jan - chrono::Duration::days(30), Some(jan));
        let s2 = make_session(book2.id, feb - chrono::Duration::days(15), Some(feb));
        let sessions = [s1, s2];

        let mut mood_data = HashMap::new();
        mood_data.insert(
            book1.id.to_string(),
            vec!["dark".to_string(), "epic".to_string()],
        );
        mood_data.insert(
            book2.id.to_string(),
            vec!["dark".to_string(), "hopeful".to_string()],
        );

        let now = "2024-03-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let stats = compute_stats(StatsInput {
            books: &books,
            sessions: &sessions,
            mood_tag_data: &mood_data,
            ..empty_input(now)
        });

        assert_eq!(stats.mood_trends.len(), 2);

        // January
        let jan_trend = &stats.mood_trends[0];
        assert_eq!(jan_trend.year, 2024);
        assert_eq!(jan_trend.month, 1);
        assert_eq!(jan_trend.moods.len(), 2);

        // February
        let feb_trend = &stats.mood_trends[1];
        assert_eq!(feb_trend.year, 2024);
        assert_eq!(feb_trend.month, 2);
        assert_eq!(feb_trend.moods.len(), 2);
    }
}
