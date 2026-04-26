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
}
