//! OPDS catalog server — serves the library to e-readers (KOReader, Moon+
//! Reader, etc.) as an OPDS 1.2 (Atom) feed.
//!
//! This is a **local-first, LAN-facing** surface: it makes no external calls
//! and serves only the local database and the ebook files already associated
//! with books via `toku file add`. It is deliberately independent from the
//! dashboard's hosted-mode (SRP) auth — the only protection here is an optional
//! HTTP Basic auth guard, enabled through the `[opds]` config section.

use std::collections::HashMap;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use toku_core::{Book, ContributorRole, OpdsConfig};
use toku_db::{BookRepository, Database};
use toku_files::{EbookFile, FileRepository};

use crate::error::WebError;
use crate::opds_xml::{
    self, ACQUISITION_TYPE, AcqEntry, AcqLink, NAVIGATION_TYPE, NavEntry, OPDS_ROOT,
    OPENSEARCH_TYPE,
};

/// Shared state for the OPDS router.
#[derive(Clone)]
pub struct OpdsState {
    pub db_path: PathBuf,
    pub covers_dir: PathBuf,
    /// Optional HTTP Basic auth credentials. `None` when auth is disabled.
    pub auth: Option<OpdsConfig>,
}

/// Everything the feeds need about one book, gathered in a single pass.
struct BookBundle {
    book: Book,
    authors: Vec<String>,
    isbns: Vec<String>,
    series: Vec<String>,
    shelves: Vec<String>,
    files: Vec<EbookFile>,
}

// ── Router / serving ─────────────────────────────────────────────────────────

/// Build the Axum router for the OPDS catalog.
pub fn build_opds_router(state: OpdsState) -> Router {
    let router = Router::new()
        .route(OPDS_ROOT, get(root_feed))
        .route(&format!("{OPDS_ROOT}/all"), get(all_feed))
        .route(&format!("{OPDS_ROOT}/authors"), get(authors_feed))
        .route(&format!("{OPDS_ROOT}/authors/{{name}}"), get(author_feed))
        .route(&format!("{OPDS_ROOT}/shelves"), get(shelves_feed))
        .route(&format!("{OPDS_ROOT}/shelves/{{name}}"), get(shelf_feed))
        .route(&format!("{OPDS_ROOT}/series"), get(series_feed))
        .route(
            &format!("{OPDS_ROOT}/series/{{name}}"),
            get(series_detail_feed),
        )
        .route(&format!("{OPDS_ROOT}/search"), get(search_feed))
        .route(&format!("{OPDS_ROOT}/opensearch.xml"), get(opensearch))
        .route(&format!("{OPDS_ROOT}/download/{{file_id}}"), get(download))
        .route(&format!("{OPDS_ROOT}/cover/{{hash}}"), get(cover))
        .route("/healthz", get(|| async { "ok" }));

    if state.auth.is_some() {
        router
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                basic_auth,
            ))
            .with_state(state)
    } else {
        router.with_state(state)
    }
}

/// Serve the OPDS catalog on `{host}:{port}`.
///
/// Runs database migrations once at startup, then serves the catalog. Unlike
/// the dashboard, the OPDS server may bind non-loopback addresses by design —
/// e-readers reach it over the LAN. Exposure beyond the LAN remains the user's
/// network/firewall responsibility (see ADR-011).
pub async fn serve_opds(
    db_path: PathBuf,
    host: &str,
    port: u16,
    auth: Option<OpdsConfig>,
) -> Result<(), WebError> {
    Database::open(&db_path)
        .map_err(|e| WebError::Internal(format!("failed to open database: {e}")))?;

    let covers_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("covers");

    let state = OpdsState {
        db_path,
        covers_dir,
        auth,
    };

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| WebError::Internal(format!("failed to bind {addr}: {e}")))?;

    let auth_note = if state.auth.is_some() {
        " (HTTP Basic auth required)"
    } else {
        ""
    };
    eprintln!("Toku OPDS catalog → http://{addr}{OPDS_ROOT}{auth_note}");

    let app = build_opds_router(state);
    axum::serve(listener, app)
        .await
        .map_err(|e| WebError::Internal(format!("server error: {e}")))?;
    Ok(())
}

// ── Auth middleware ──────────────────────────────────────────────────────────

/// HTTP Basic auth guard. Mounted only when `[opds]` credentials are set.
async fn basic_auth(
    State(state): State<OpdsState>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let Some(cfg) = &state.auth else {
        return next.run(req).await;
    };

    if let Some((user, pass)) = extract_basic_credentials(&req) {
        let user_ok = cfg.username.as_deref() == Some(user.as_str());
        // Always run the (constant-time) password verification to avoid leaking
        // whether the username was correct via timing.
        let pass_ok = cfg.verify_password(&pass);
        if user_ok && pass_ok {
            return next.run(req).await;
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            "Basic realm=\"Toku OPDS\", charset=\"UTF-8\"",
        )],
        "Unauthorized",
    )
        .into_response()
}

/// Parse a `Authorization: Basic <base64(user:pass)>` header.
fn extract_basic_credentials(req: &axum::extract::Request) -> Option<(String, String)> {
    use base64::Engine;
    let header = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = header
        .strip_prefix("Basic ")
        .or_else(|| header.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, pass) = decoded.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /opds` — root navigation feed.
async fn root_feed() -> Response {
    let updated = now_rfc3339();
    let entries = vec![
        NavEntry {
            title: "All Books".into(),
            href: format!("{OPDS_ROOT}/all"),
            content: Some("Every book with a downloadable file".into()),
            acquisition: true,
        },
        NavEntry {
            title: "By Author".into(),
            href: format!("{OPDS_ROOT}/authors"),
            content: Some("Browse by author".into()),
            acquisition: false,
        },
        NavEntry {
            title: "By Series".into(),
            href: format!("{OPDS_ROOT}/series"),
            content: Some("Browse by series".into()),
            acquisition: false,
        },
        NavEntry {
            title: "By Shelf".into(),
            href: format!("{OPDS_ROOT}/shelves"),
            content: Some("Browse by shelf".into()),
            acquisition: false,
        },
    ];
    let feed = opds_xml::navigation_feed(
        "urn:toku:opds",
        "Toku Library",
        OPDS_ROOT,
        &updated,
        &entries,
    );
    feed_response(NAVIGATION_TYPE, feed)
}

/// `GET /opds/all` — acquisition feed of every book with files.
async fn all_feed(State(state): State<OpdsState>) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let entries = bundles.iter().map(bundle_to_entry).collect::<Vec<_>>();
    let feed = opds_xml::acquisition_feed(
        "urn:toku:opds:all",
        "All Books",
        &format!("{OPDS_ROOT}/all"),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(ACQUISITION_TYPE, feed))
}

/// `GET /opds/authors` — navigation feed of authors (with book counts).
async fn authors_feed(State(state): State<OpdsState>) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let entries = grouped_nav(&bundles, "authors", |b| b.authors.clone());
    let feed = opds_xml::navigation_feed(
        "urn:toku:opds:authors",
        "Authors",
        &format!("{OPDS_ROOT}/authors"),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(NAVIGATION_TYPE, feed))
}

/// `GET /opds/authors/{name}` — acquisition feed for one author.
async fn author_feed(
    State(state): State<OpdsState>,
    Path(name): Path<String>,
) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let matches = filter_bundles(&bundles, &name, |b| &b.authors);
    let entries = matches
        .iter()
        .map(|b| bundle_to_entry(b))
        .collect::<Vec<_>>();
    let feed = opds_xml::acquisition_feed(
        &format!("urn:toku:opds:author:{name}"),
        &name,
        &format!("{OPDS_ROOT}/authors/{}", urlencoding::encode(&name)),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(ACQUISITION_TYPE, feed))
}

/// `GET /opds/series` — navigation feed of series.
async fn series_feed(State(state): State<OpdsState>) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let entries = grouped_nav(&bundles, "series", |b| b.series.clone());
    let feed = opds_xml::navigation_feed(
        "urn:toku:opds:series",
        "Series",
        &format!("{OPDS_ROOT}/series"),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(NAVIGATION_TYPE, feed))
}

/// `GET /opds/series/{name}` — acquisition feed for one series.
async fn series_detail_feed(
    State(state): State<OpdsState>,
    Path(name): Path<String>,
) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let matches = filter_bundles(&bundles, &name, |b| &b.series);
    let entries = matches
        .iter()
        .map(|b| bundle_to_entry(b))
        .collect::<Vec<_>>();
    let feed = opds_xml::acquisition_feed(
        &format!("urn:toku:opds:series:{name}"),
        &name,
        &format!("{OPDS_ROOT}/series/{}", urlencoding::encode(&name)),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(ACQUISITION_TYPE, feed))
}

/// `GET /opds/shelves` — navigation feed of shelves.
async fn shelves_feed(State(state): State<OpdsState>) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let entries = grouped_nav(&bundles, "shelves", |b| b.shelves.clone());
    let feed = opds_xml::navigation_feed(
        "urn:toku:opds:shelves",
        "Shelves",
        &format!("{OPDS_ROOT}/shelves"),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(NAVIGATION_TYPE, feed))
}

/// `GET /opds/shelves/{name}` — acquisition feed for one shelf.
async fn shelf_feed(
    State(state): State<OpdsState>,
    Path(name): Path<String>,
) -> Result<Response, WebError> {
    let bundles = load_bundles(&state.db_path).await?;
    let matches = filter_bundles(&bundles, &name, |b| &b.shelves);
    let entries = matches
        .iter()
        .map(|b| bundle_to_entry(b))
        .collect::<Vec<_>>();
    let feed = opds_xml::acquisition_feed(
        &format!("urn:toku:opds:shelf:{name}"),
        &name,
        &format!("{OPDS_ROOT}/shelves/{}", urlencoding::encode(&name)),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(ACQUISITION_TYPE, feed))
}

#[derive(serde::Deserialize)]
struct SearchParams {
    q: Option<String>,
}

/// `GET /opds/search?q=` — acquisition feed of search results (files only).
async fn search_feed(
    State(state): State<OpdsState>,
    Query(params): Query<SearchParams>,
) -> Result<Response, WebError> {
    let query = params.q.unwrap_or_default();
    let bundles = load_bundles(&state.db_path).await?;

    let entries = if query.trim().is_empty() {
        Vec::new()
    } else {
        let db_path = state.db_path.clone();
        let q = query.clone();
        let ids = tokio::task::spawn_blocking(move || -> Result<Vec<String>, WebError> {
            let db = Database::open_no_migrate(&db_path)?;
            let repo = BookRepository::new(&db);
            Ok(repo
                .search_books_filtered(&q, None, None, None)?
                .into_iter()
                .map(|b| b.id.to_string())
                .collect())
        })
        .await
        .map_err(|e| WebError::Internal(e.to_string()))??;

        let idset: std::collections::HashSet<String> = ids.into_iter().collect();
        bundles
            .iter()
            .filter(|b| idset.contains(&b.book.id.to_string()))
            .map(bundle_to_entry)
            .collect()
    };

    let feed = opds_xml::acquisition_feed(
        "urn:toku:opds:search",
        &format!("Search: {query}"),
        &format!("{OPDS_ROOT}/search?q={}", urlencoding::encode(&query)),
        &now_rfc3339(),
        &entries,
    );
    Ok(feed_response(ACQUISITION_TYPE, feed))
}

/// `GET /opds/opensearch.xml` — OpenSearch description document.
async fn opensearch() -> Response {
    feed_response(OPENSEARCH_TYPE, opds_xml::opensearch_description())
}

/// `GET /opds/download/{file_id}` — stream an associated ebook file from disk.
async fn download(
    State(state): State<OpdsState>,
    Path(file_id): Path<String>,
) -> Result<Response, WebError> {
    let id = uuid::Uuid::parse_str(&file_id)
        .map_err(|_| WebError::NotFound("invalid file id".into()))?;
    let db_path = state.db_path.clone();

    let file = tokio::task::spawn_blocking(move || -> Result<Option<EbookFile>, WebError> {
        let db = Database::open_no_migrate(&db_path)?;
        let repo = FileRepository::new(&db);
        repo.get_file(&id)
            .map_err(|e| WebError::Internal(e.to_string()))
    })
    .await
    .map_err(|e| WebError::Internal(e.to_string()))??
    .ok_or_else(|| WebError::NotFound("file not found".into()))?;

    // Serve only the exact stored path — never a client-supplied path.
    let path = PathBuf::from(&file.path);
    let data = tokio::task::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?
        .map_err(|_| WebError::NotFound("file missing on disk".into()))?;

    let filename = std::path::Path::new(&file.path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("book")
        .to_string();
    let disposition = format!(
        "attachment; filename=\"{}\"",
        filename.replace('"', "").replace(['\r', '\n'], "")
    );

    Ok((
        [
            (
                header::CONTENT_TYPE,
                opds_xml::format_mime(file.format).to_string(),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        Body::from(data),
    )
        .into_response())
}

/// `GET /opds/cover/{hash}` — serve a cover image from disk.
async fn cover(
    State(state): State<OpdsState>,
    Path(hash): Path<String>,
) -> Result<Response, WebError> {
    if hash.is_empty() || hash.len() > 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(WebError::NotFound("invalid cover hash".into()));
    }
    let path = state.covers_dir.join(format!("{hash}.jpg"));
    let data = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?
        .map_err(|_| WebError::NotFound("cover not found".into()))?;

    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        data,
    )
        .into_response())
}

// ── Data gathering ───────────────────────────────────────────────────────────

/// Load every book that has ≥1 associated file, with the metadata the feeds
/// need. Runs the blocking DB work off the async runtime.
async fn load_bundles(db_path: &std::path::Path) -> Result<Vec<BookBundle>, WebError> {
    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || gather_bundles(&db_path))
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?
}

fn gather_bundles(db_path: &std::path::Path) -> Result<Vec<BookBundle>, WebError> {
    let db = Database::open_no_migrate(db_path)?;
    let repo = BookRepository::new(&db);
    let file_repo = FileRepository::new(&db);

    // Group all files by book.
    let mut files_by_book: HashMap<String, Vec<EbookFile>> = HashMap::new();
    for f in file_repo
        .list_all_files()
        .map_err(|e| WebError::Internal(e.to_string()))?
    {
        files_by_book
            .entry(f.book_id.to_string())
            .or_default()
            .push(f);
    }

    let mut bundles = Vec::new();
    for book in repo.list_books()? {
        let Some(files) = files_by_book.remove(&book.id.to_string()) else {
            continue; // no files → not served over OPDS
        };

        let authors = repo
            .get_book_authors(&book.id)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, ba)| ba.role == ContributorRole::Author)
            .map(|(a, _)| a.name)
            .collect();
        let isbns = repo.get_book_isbns(&book.id).unwrap_or_default();
        let series = repo
            .get_book_series(&book.id)
            .unwrap_or_default()
            .into_iter()
            .map(|(s, _)| s.name)
            .collect();
        let shelves = repo
            .get_book_shelves(&book.id)
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name)
            .collect();

        bundles.push(BookBundle {
            book,
            authors,
            isbns,
            series,
            shelves,
            files,
        });
    }

    // Stable ordering by title for deterministic feeds.
    bundles.sort_by(|a, b| {
        a.book
            .title
            .to_lowercase()
            .cmp(&b.book.title.to_lowercase())
    });
    Ok(bundles)
}

/// Convert a book bundle into an OPDS acquisition entry.
fn bundle_to_entry(b: &BookBundle) -> AcqEntry {
    let mut files = b.files.clone();
    files.sort_by_key(|f| f.format.as_str());
    let links = files
        .iter()
        .map(|f| AcqLink {
            href: format!("{OPDS_ROOT}/download/{}", f.id),
            mime: opds_xml::format_mime(f.format).to_string(),
        })
        .collect();

    let cover_href = b
        .book
        .cover_hash
        .as_ref()
        .map(|h| format!("{OPDS_ROOT}/cover/{h}"));

    AcqEntry {
        id: format!("urn:uuid:{}", b.book.id),
        title: b.book.title.clone(),
        authors: b.authors.clone(),
        updated: b.book.updated_at.to_rfc3339(),
        summary: b.book.description.clone(),
        language: b.book.language.clone(),
        isbns: b.isbns.clone(),
        publisher_year: b.book.pub_date.clone(),
        cover_href,
        links,
    }
}

/// Build a navigation feed grouping bundles by a multi-valued key (author,
/// series, shelf), producing one nav entry per distinct value with a count.
fn grouped_nav(
    bundles: &[BookBundle],
    segment: &str,
    key: impl Fn(&BookBundle) -> Vec<String>,
) -> Vec<NavEntry> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for b in bundles {
        for value in key(b) {
            if value.trim().is_empty() {
                continue;
            }
            *counts.entry(value).or_default() += 1;
        }
    }
    let mut names: Vec<(String, usize)> = counts.into_iter().collect();
    names.sort_by_key(|(name, _)| name.to_lowercase());

    names
        .into_iter()
        .map(|(name, count)| NavEntry {
            title: name.clone(),
            href: format!("{OPDS_ROOT}/{segment}/{}", urlencoding::encode(&name)),
            content: Some(format!("{count} book{}", if count == 1 { "" } else { "s" })),
            acquisition: true,
        })
        .collect()
}

/// Filter bundles whose multi-valued key contains `name` (case-insensitive).
fn filter_bundles<'a>(
    bundles: &'a [BookBundle],
    name: &str,
    key: impl Fn(&BookBundle) -> &Vec<String>,
) -> Vec<&'a BookBundle> {
    let needle = name.to_lowercase();
    bundles
        .iter()
        .filter(|b| key(b).iter().any(|v| v.to_lowercase() == needle))
        .collect()
}

// ── Small helpers ────────────────────────────────────────────────────────────

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn feed_response(content_type: &str, body: String) -> Response {
    ([(header::CONTENT_TYPE, content_type.to_string())], body).into_response()
}
