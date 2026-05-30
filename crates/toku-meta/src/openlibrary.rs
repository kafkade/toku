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

/// A single result from an Open Library search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub authors: Vec<String>,
    pub first_publish_year: Option<i32>,
    pub edition_count: i32,
    pub isbn: Option<String>,
    pub page_count: Option<i32>,
    pub openlibrary_key: String,
    pub languages: Vec<String>,
}

/// Search Open Library by free-text query. Returns up to `limit` results.
pub async fn search_books(query: &str, limit: usize) -> Result<Vec<SearchResult>, MetaError> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;

    let resp = client
        .get("https://openlibrary.org/search.json")
        .query(&[
            ("q", query),
            ("limit", &limit.to_string()),
            ("fields", "title,author_name,first_publish_year,isbn,edition_count,key,number_of_pages_median,language"),
        ])
        .send()
        .await?
        .error_for_status()?;
    let raw: RawSearchResponse = resp.json().await?;

    let results = raw
        .docs
        .into_iter()
        .map(|doc| {
            // Pick the first ISBN-13 if available, else first ISBN-10
            let isbn = doc.isbn.as_ref().and_then(|isbns| {
                isbns
                    .iter()
                    .find(|i| i.len() == 13)
                    .or(isbns.first())
                    .cloned()
            });

            SearchResult {
                title: doc.title,
                authors: doc.author_name.unwrap_or_default(),
                first_publish_year: doc.first_publish_year,
                edition_count: doc.edition_count.unwrap_or(0),
                isbn,
                page_count: doc.number_of_pages_median,
                openlibrary_key: doc.key,
                languages: doc.language.unwrap_or_default(),
            }
        })
        .collect();

    Ok(results)
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
///
/// This function is intended for user-initiated single-book enrichment only
/// (e.g., `toku add --isbn`). Callers must NOT use this for bulk pre-fetching
/// or crawling — Open Library's Covers API prohibits that usage and rate-limits
/// to 100 requests per 5 minutes for ISBN-based lookups.
///
/// Cover images from Open Library: on-demand local caching is permitted per OL's
/// API guidelines. See `docs/validations/cover-image-licensing.md`.
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
struct RawSearchResponse {
    docs: Vec<RawSearchDoc>,
}

#[derive(Deserialize)]
struct RawSearchDoc {
    title: String,
    author_name: Option<Vec<String>>,
    first_publish_year: Option<i32>,
    edition_count: Option<i32>,
    isbn: Option<Vec<String>>,
    key: String,
    number_of_pages_median: Option<i32>,
    language: Option<Vec<String>>,
}

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
