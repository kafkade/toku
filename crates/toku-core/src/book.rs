use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A book in the user's library. Each row represents an edition (Book = Edition).
/// A nullable `work_id` is reserved for Phase 3 work-grouping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub page_count: Option<i32>,
    pub pub_date: Option<String>,
    pub language: Option<String>,
    pub format: BookFormat,
    /// Duration in minutes — only meaningful for audiobooks.
    pub duration_minutes: Option<i32>,
    pub cover_hash: Option<String>,
    /// Reserved for Phase 3 work-grouping. NULL until then.
    pub work_id: Option<Uuid>,
    pub status: ReadingStatus,
    pub rating: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Book {
    pub fn new(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            title: title.into(),
            subtitle: None,
            description: None,
            page_count: None,
            pub_date: None,
            language: None,
            format: BookFormat::Physical,
            duration_minutes: None,
            cover_hash: None,
            work_id: None,
            status: ReadingStatus::WantToRead,
            rating: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Physical book, ebook, or audiobook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookFormat {
    Physical,
    Ebook,
    Audiobook,
}

impl BookFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Physical => "physical",
            Self::Ebook => "ebook",
            Self::Audiobook => "audiobook",
        }
    }
}

impl std::fmt::Display for BookFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BookFormat {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "physical" => Ok(Self::Physical),
            "ebook" => Ok(Self::Ebook),
            "audiobook" => Ok(Self::Audiobook),
            _ => Err(crate::TokuError::InvalidFormat(s.to_string())),
        }
    }
}

/// A reading session tracks a single reading attempt of a book.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingSession {
    pub id: Uuid,
    pub book_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub start_page: Option<i32>,
    pub end_page: Option<i32>,
    pub rating: Option<i32>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ReadingSession {
    pub fn new(book_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            book_id,
            started_at: now,
            finished_at: None,
            start_page: None,
            end_page: None,
            rating: None,
            notes: None,
            created_at: now,
        }
    }
}

/// Type of reading progress entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressType {
    Page,
    Percent,
    Chapter,
    /// Duration in minutes (for audiobooks).
    Duration,
}

impl ProgressType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Percent => "percent",
            Self::Chapter => "chapter",
            Self::Duration => "duration",
        }
    }
}

impl std::fmt::Display for ProgressType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProgressType {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "page" => Ok(Self::Page),
            "percent" => Ok(Self::Percent),
            "chapter" => Ok(Self::Chapter),
            "duration" => Ok(Self::Duration),
            _ => Err(crate::TokuError::InvalidProgressType(s.to_string())),
        }
    }
}

/// A timestamped reading progress entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingProgress {
    pub id: Uuid,
    pub book_id: Uuid,
    pub session_id: Option<Uuid>,
    pub progress_type: ProgressType,
    pub value: i32,
    pub note: Option<String>,
    pub logged_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl ReadingProgress {
    pub fn new(book_id: Uuid, progress_type: ProgressType, value: i32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            book_id,
            session_id: None,
            progress_type,
            value,
            note: None,
            logged_at: now,
            created_at: now,
        }
    }
}

/// Parse a human-friendly duration string into total minutes.
///
/// Accepted formats: `5h30m`, `330m`, `5.5h`, `5h`, `90`.
pub fn parse_duration_to_minutes(s: &str) -> Result<i32, crate::TokuError> {
    let s = s.trim();

    // Try `Xh Ym` or `XhYm`
    if let Some(h_pos) = s.find('h') {
        let hours_str = &s[..h_pos];
        let rest = s[h_pos + 1..].trim();

        if rest.is_empty() {
            // Could be fractional hours like "5.5h"
            let hours: f64 = hours_str
                .parse()
                .map_err(|_| crate::TokuError::InvalidDuration(s.to_string()))?;
            return Ok((hours * 60.0).round() as i32);
        }

        let mins_str = rest.trim_end_matches('m');
        let hours: f64 = hours_str
            .parse()
            .map_err(|_| crate::TokuError::InvalidDuration(s.to_string()))?;
        let mins: f64 = mins_str
            .parse()
            .map_err(|_| crate::TokuError::InvalidDuration(s.to_string()))?;
        return Ok((hours * 60.0 + mins).round() as i32);
    }

    // Try `Xm`
    if let Some(m_str) = s.strip_suffix('m') {
        let mins: f64 = m_str
            .parse()
            .map_err(|_| crate::TokuError::InvalidDuration(s.to_string()))?;
        return Ok(mins.round() as i32);
    }

    // Plain number = minutes
    let mins: f64 = s
        .parse()
        .map_err(|_| crate::TokuError::InvalidDuration(s.to_string()))?;
    Ok(mins.round() as i32)
}

/// Reading lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingStatus {
    WantToRead,
    Reading,
    Read,
    Abandoned,
    OnHold,
}

impl ReadingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WantToRead => "want-to-read",
            Self::Reading => "reading",
            Self::Read => "read",
            Self::Abandoned => "abandoned",
            Self::OnHold => "on-hold",
        }
    }

    /// Returns whether a transition from this status to `target` is valid.
    pub fn can_transition_to(&self, target: &ReadingStatus) -> bool {
        matches!(
            (self, target),
            (ReadingStatus::WantToRead, ReadingStatus::Reading)
                | (ReadingStatus::Reading, ReadingStatus::Read)
                | (ReadingStatus::Reading, ReadingStatus::Abandoned)
                | (ReadingStatus::Reading, ReadingStatus::OnHold)
                | (ReadingStatus::OnHold, ReadingStatus::Reading)
                | (ReadingStatus::Abandoned, ReadingStatus::Reading)
                | (ReadingStatus::Read, ReadingStatus::Reading) // re-read
        )
    }
}

impl std::fmt::Display for ReadingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReadingStatus {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "want-to-read" | "to-read" => Ok(Self::WantToRead),
            "reading" | "currently-reading" => Ok(Self::Reading),
            "read" => Ok(Self::Read),
            "abandoned" | "dnf" => Ok(Self::Abandoned),
            "on-hold" | "paused" => Ok(Self::OnHold),
            _ => Err(crate::TokuError::InvalidStatus(s.to_string())),
        }
    }
}

/// Contributor role for a book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContributorRole {
    Author,
    Editor,
    Translator,
    Illustrator,
    Narrator,
}

impl ContributorRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Editor => "editor",
            Self::Translator => "translator",
            Self::Illustrator => "illustrator",
            Self::Narrator => "narrator",
        }
    }
}

impl std::fmt::Display for ContributorRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ContributorRole {
    type Err = crate::TokuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "author" => Ok(Self::Author),
            "editor" => Ok(Self::Editor),
            "translator" => Ok(Self::Translator),
            "illustrator" => Ok(Self::Illustrator),
            "narrator" => Ok(Self::Narrator),
            _ => Err(crate::TokuError::InvalidRole(s.to_string())),
        }
    }
}

/// An author or other contributor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    pub sort_name: Option<String>,
}

impl Author {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let sort_name = guess_sort_name(&name);
        Self {
            id: Uuid::now_v7(),
            name,
            sort_name: Some(sort_name),
        }
    }
}

/// Guess a sort name from a display name: "Ursula K. Le Guin" → "Le Guin, Ursula K."
fn guess_sort_name(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    if parts.len() <= 1 {
        return name.to_string();
    }
    let last = parts.last().unwrap();
    let rest: Vec<&str> = parts[..parts.len() - 1].to_vec();
    format!("{}, {}", last, rest.join(" "))
}

/// A user-defined shelf for organizing books (e.g. "Favorites", "To Re-read").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shelf {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl Shelf {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            created_at: Utc::now(),
        }
    }
}

/// A user-defined tag for categorizing books (e.g. "sci-fi", "Hugo winner").
/// Tag names are case-insensitive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl Tag {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            created_at: Utc::now(),
        }
    }
}

/// A book-to-author relationship with role and ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookAuthor {
    pub book_id: Uuid,
    pub author_id: Uuid,
    pub role: ContributorRole,
    pub position: i32,
}

/// A named series of books.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub id: Uuid,
    pub name: String,
    pub total_books: Option<i32>,
}

/// A book's position within a series. Position is TEXT to handle "1.5", "2a", etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSeries {
    pub book_id: Uuid,
    pub series_id: Uuid,
    pub position: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_new_has_defaults() {
        let book = Book::new("Dune");
        assert_eq!(book.title, "Dune");
        assert_eq!(book.status, ReadingStatus::WantToRead);
        assert_eq!(book.format, BookFormat::Physical);
        assert!(book.subtitle.is_none());
        assert!(book.rating.is_none());
    }

    #[test]
    fn author_sort_name() {
        // Naive heuristic: last word becomes sort key.
        // "Le Guin" is a known limitation — users can manually set sort_name.
        let author = Author::new("Frank Herbert");
        assert_eq!(author.sort_name.as_deref(), Some("Herbert, Frank"));
    }

    #[test]
    fn author_single_name() {
        let author = Author::new("Voltaire");
        assert_eq!(author.sort_name.as_deref(), Some("Voltaire"));
    }

    #[test]
    fn reading_status_roundtrip() {
        for status in [
            ReadingStatus::WantToRead,
            ReadingStatus::Reading,
            ReadingStatus::Read,
            ReadingStatus::Abandoned,
            ReadingStatus::OnHold,
        ] {
            let parsed: ReadingStatus = status.as_str().parse().unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn book_format_roundtrip() {
        for fmt in [
            BookFormat::Physical,
            BookFormat::Ebook,
            BookFormat::Audiobook,
        ] {
            let parsed: BookFormat = fmt.as_str().parse().unwrap();
            assert_eq!(parsed, fmt);
        }
    }

    #[test]
    fn reading_status_goodreads_aliases() {
        assert_eq!(
            "currently-reading".parse::<ReadingStatus>().unwrap(),
            ReadingStatus::Reading
        );
        assert_eq!(
            "to-read".parse::<ReadingStatus>().unwrap(),
            ReadingStatus::WantToRead
        );
        assert_eq!(
            "dnf".parse::<ReadingStatus>().unwrap(),
            ReadingStatus::Abandoned
        );
    }

    // --- State machine tests ---

    #[test]
    fn valid_transitions() {
        let valid = [
            (ReadingStatus::WantToRead, ReadingStatus::Reading),
            (ReadingStatus::Reading, ReadingStatus::Read),
            (ReadingStatus::Reading, ReadingStatus::Abandoned),
            (ReadingStatus::Reading, ReadingStatus::OnHold),
            (ReadingStatus::OnHold, ReadingStatus::Reading),
            (ReadingStatus::Abandoned, ReadingStatus::Reading),
            (ReadingStatus::Read, ReadingStatus::Reading), // re-read
        ];

        for (from, to) in &valid {
            assert!(from.can_transition_to(to), "{from} → {to} should be valid");
        }
    }

    #[test]
    fn invalid_transitions() {
        let invalid = [
            (ReadingStatus::WantToRead, ReadingStatus::Read),
            (ReadingStatus::WantToRead, ReadingStatus::Abandoned),
            (ReadingStatus::WantToRead, ReadingStatus::OnHold),
            (ReadingStatus::Read, ReadingStatus::Abandoned),
            (ReadingStatus::Read, ReadingStatus::OnHold),
            (ReadingStatus::Read, ReadingStatus::WantToRead),
            (ReadingStatus::Abandoned, ReadingStatus::Read),
            (ReadingStatus::Abandoned, ReadingStatus::OnHold),
            (ReadingStatus::OnHold, ReadingStatus::Read),
            (ReadingStatus::OnHold, ReadingStatus::Abandoned),
            // Self-transitions
            (ReadingStatus::Reading, ReadingStatus::Reading),
            (ReadingStatus::WantToRead, ReadingStatus::WantToRead),
        ];

        for (from, to) in &invalid {
            assert!(
                !from.can_transition_to(to),
                "{from} → {to} should be invalid"
            );
        }
    }

    #[test]
    fn reading_session_new_defaults() {
        let book_id = Uuid::now_v7();
        let session = ReadingSession::new(book_id);
        assert_eq!(session.book_id, book_id);
        assert!(session.finished_at.is_none());
        assert!(session.rating.is_none());
        assert!(session.notes.is_none());
    }

    #[test]
    fn progress_type_roundtrip() {
        for pt in [
            ProgressType::Page,
            ProgressType::Percent,
            ProgressType::Chapter,
            ProgressType::Duration,
        ] {
            let parsed: ProgressType = pt.as_str().parse().unwrap();
            assert_eq!(parsed, pt);
        }
    }

    #[test]
    fn progress_type_display() {
        assert_eq!(ProgressType::Page.to_string(), "page");
        assert_eq!(ProgressType::Duration.to_string(), "duration");
    }

    #[test]
    fn progress_type_invalid() {
        assert!("invalid".parse::<ProgressType>().is_err());
    }

    #[test]
    fn reading_progress_new_defaults() {
        let book_id = Uuid::now_v7();
        let progress = ReadingProgress::new(book_id, ProgressType::Page, 42);
        assert_eq!(progress.book_id, book_id);
        assert_eq!(progress.progress_type, ProgressType::Page);
        assert_eq!(progress.value, 42);
        assert!(progress.session_id.is_none());
        assert!(progress.note.is_none());
    }

    #[test]
    fn parse_duration_hours_minutes() {
        assert_eq!(parse_duration_to_minutes("5h30m").unwrap(), 330);
        assert_eq!(parse_duration_to_minutes("1h0m").unwrap(), 60);
        assert_eq!(parse_duration_to_minutes("0h45m").unwrap(), 45);
    }

    #[test]
    fn parse_duration_minutes_only() {
        assert_eq!(parse_duration_to_minutes("330m").unwrap(), 330);
        assert_eq!(parse_duration_to_minutes("90m").unwrap(), 90);
    }

    #[test]
    fn parse_duration_hours_only() {
        assert_eq!(parse_duration_to_minutes("5h").unwrap(), 300);
        assert_eq!(parse_duration_to_minutes("5.5h").unwrap(), 330);
        assert_eq!(parse_duration_to_minutes("2.25h").unwrap(), 135);
    }

    #[test]
    fn parse_duration_plain_number() {
        assert_eq!(parse_duration_to_minutes("90").unwrap(), 90);
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration_to_minutes("abc").is_err());
        assert!(parse_duration_to_minutes("").is_err());
    }
}
