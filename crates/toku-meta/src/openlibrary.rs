use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::MetaError;

const USER_AGENT: &str = "toku/0.1.0 (https://github.com/kafkade/toku)";

/// Parsed response from Open Library's ISBN endpoint.
#[derive(Debug, Clone)]
pub struct OpenLibraryBook {
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub page_count: Option<i32>,
    pub pub_date: Option<String>,
    pub language: Option<String>,
    pub publishers: Vec<String>,
    pub authors: Vec<String>,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub openlibrary_id: Option<String>,
    pub cover_id: Option<i64>,
}

/// Fetch book metadata from Open Library by ISBN.
pub async fn fetch_by_isbn(isbn: &str) -> Result<OpenLibraryBook, MetaError> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;

    let url = format!("https://openlibrary.org/isbn/{isbn}.json");
    let resp = client.get(&url).send().await?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(MetaError::NotFound(isbn.to_string()));
    }

    let resp = resp.error_for_status()?;
    let raw: RawEdition = resp.json().await?;

    // Resolve author names from author keys
    let mut authors = Vec::new();
    for author_ref in &raw.authors.unwrap_or_default() {
        if let Some(key) = &author_ref.key
            && let Ok(name) = fetch_author_name(&client, key).await
        {
            authors.push(name);
        }
    }

    let description = raw.description.map(|d| match d {
        DescriptionField::Simple(s) => s,
        DescriptionField::Object { value } => value,
    });

    Ok(OpenLibraryBook {
        title: raw.title,
        subtitle: raw.subtitle,
        description,
        page_count: raw.number_of_pages,
        pub_date: raw.publish_date,
        language: raw
            .languages
            .and_then(|langs| langs.first().and_then(|l| l.key.clone()))
            .map(|k| k.replace("/languages/", "")),
        publishers: raw.publishers.unwrap_or_default(),
        authors,
        isbn_13: raw.isbn_13.and_then(|v| v.into_iter().next()),
        isbn_10: raw.isbn_10.and_then(|v| v.into_iter().next()),
        openlibrary_id: raw.key,
        cover_id: raw.covers.and_then(|c| c.into_iter().next()),
    })
}

/// Download a cover image and save it content-addressed by SHA-256.
/// Returns the hash filename (e.g., "a1b2c3d4e5f6.jpg").
pub async fn fetch_cover(isbn: &str, covers_dir: &Path) -> Result<Option<String>, MetaError> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;

    let url = format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false");
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let bytes = resp.bytes().await?;

    // Open Library returns a 1x1 pixel placeholder for missing covers
    if bytes.len() < 1000 {
        return Ok(None);
    }

    let hash = Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut acc, b| {
            use std::fmt::Write;
            write!(acc, "{b:02x}").unwrap();
            acc
        });
    let hash_short = &hash[..16];
    let filename = format!("{hash_short}.jpg");

    std::fs::create_dir_all(covers_dir)?;
    let path = covers_dir.join(&filename);
    std::fs::write(&path, &bytes)?;

    Ok(Some(hash_short.to_string()))
}

async fn fetch_author_name(client: &reqwest::Client, key: &str) -> Result<String, MetaError> {
    let url = format!("https://openlibrary.org{key}.json");
    let resp = client.get(&url).send().await?.error_for_status()?;
    let author: RawAuthor = resp.json().await?;
    Ok(author.name)
}

// --- Raw API response structs ---

#[derive(Deserialize)]
struct RawEdition {
    title: String,
    subtitle: Option<String>,
    description: Option<DescriptionField>,
    number_of_pages: Option<i32>,
    publish_date: Option<String>,
    publishers: Option<Vec<String>>,
    authors: Option<Vec<AuthorRef>>,
    languages: Option<Vec<LanguageRef>>,
    isbn_13: Option<Vec<String>>,
    isbn_10: Option<Vec<String>>,
    covers: Option<Vec<i64>>,
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DescriptionField {
    Simple(String),
    Object { value: String },
}

#[derive(Deserialize)]
struct AuthorRef {
    key: Option<String>,
}

#[derive(Deserialize)]
struct LanguageRef {
    key: Option<String>,
}

#[derive(Deserialize)]
struct RawAuthor {
    name: String,
}
