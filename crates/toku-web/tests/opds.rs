//! Integration tests for the OPDS catalog server (issue #150).
//!
//! These drive the OPDS router end to end via `tower::ServiceExt::oneshot`,
//! covering feed shape, browse-by-* grouping, search, downloads that honor file
//! associations, and the optional HTTP Basic auth guard.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use http_body_util::BodyExt;
use tower::ServiceExt;

use toku_core::{Book, OpdsConfig};
use toku_db::{BookRepository, Database};
use toku_files::{EbookFile, FileFormat, FileRepository, sha256_file};
use toku_web::opds::{OpdsState, build_opds_router};

struct TestEnv {
    _dir: tempfile::TempDir,
    db_path: std::path::PathBuf,
    covers_dir: std::path::PathBuf,
}

/// Build a library with one book that has an associated epub file and one book
/// with no files. Returns the env plus the added file's id.
fn setup_env() -> (TestEnv, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("toku.db");
    let covers_dir = dir.path().join("covers");
    std::fs::create_dir_all(&covers_dir).expect("covers dir");

    let db = Database::open(&db_path).expect("migrate db");
    let repo = BookRepository::new(&db);

    // Book with a file.
    let mut dune = Book::new("Dune");
    dune.description = Some("Desert planet".into());
    dune.language = Some("en".into());
    repo.create_book(&dune).expect("create dune");
    let author = toku_core::Author::new("Frank Herbert");
    repo.add_book_author(&author, &dune.id, toku_core::ContributorRole::Author, 0)
        .expect("add author");
    repo.add_isbn("9780441172719", &dune.id).expect("add isbn");
    repo.create_shelf("Favorites").expect("create shelf");
    repo.add_book_to_shelf(&dune.id, "Favorites")
        .expect("add to shelf");

    // Book without any file — must NOT appear in acquisition feeds.
    let neuromancer = Book::new("Neuromancer");
    repo.create_book(&neuromancer).expect("create neuromancer");

    // Associate a real file on disk with Dune.
    let epub_path = dir.path().join("dune.epub");
    std::fs::write(&epub_path, b"epub-bytes").expect("write epub");
    let checksum = sha256_file(&epub_path).expect("checksum");
    let file = EbookFile::new(
        dune.id,
        epub_path.to_string_lossy().into_owned(),
        FileFormat::Epub,
        10,
        checksum,
    );
    let file_id = file.id.to_string();
    FileRepository::new(&db).add_file(&file).expect("add file");

    (
        TestEnv {
            _dir: dir,
            db_path,
            covers_dir,
        },
        file_id,
    )
}

fn router(env: &TestEnv, auth: Option<OpdsConfig>) -> Router {
    build_opds_router(OpdsState {
        db_path: env.db_path.clone(),
        covers_dir: env.covers_dir.clone(),
        auth,
    })
}

async fn get(router: Router, uri: &str) -> (StatusCode, String, Option<String>) {
    let resp = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        String::from_utf8_lossy(&body).into_owned(),
        content_type,
    )
}

#[tokio::test]
async fn root_feed_is_navigation_with_subsections() {
    let (env, _) = setup_env();
    let (status, body, ct) = get(router(&env, None), "/opds").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.unwrap().contains("kind=navigation"));
    assert!(body.contains("<title>All Books</title>"));
    assert!(body.contains("href=\"/opds/authors\""));
    assert!(body.contains("href=\"/opds/series\""));
    assert!(body.contains("href=\"/opds/shelves\""));
    assert!(body.contains("rel=\"search\""));
}

#[tokio::test]
async fn all_feed_only_lists_books_with_files() {
    let (env, file_id) = setup_env();
    let (status, body, ct) = get(router(&env, None), "/opds/all").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.unwrap().contains("kind=acquisition"));
    assert!(body.contains("<title>Dune</title>"));
    assert!(body.contains("<name>Frank Herbert</name>"));
    assert!(body.contains("urn:isbn:9780441172719"));
    assert!(body.contains(&format!("/opds/download/{file_id}")));
    // Neuromancer has no file and must be excluded.
    assert!(!body.contains("Neuromancer"));
}

#[tokio::test]
async fn browse_by_author_shelf_and_series() {
    let (env, _) = setup_env();

    let (_, authors, _) = get(router(&env, None), "/opds/authors").await;
    assert!(authors.contains("<title>Frank Herbert</title>"));
    assert!(authors.contains("href=\"/opds/authors/Frank%20Herbert\""));

    let (_, author_detail, _) = get(router(&env, None), "/opds/authors/Frank%20Herbert").await;
    assert!(author_detail.contains("<title>Dune</title>"));

    let (_, shelves, _) = get(router(&env, None), "/opds/shelves").await;
    assert!(shelves.contains("<title>Favorites</title>"));

    let (_, shelf_detail, _) = get(router(&env, None), "/opds/shelves/Favorites").await;
    assert!(shelf_detail.contains("<title>Dune</title>"));

    // No series configured → empty (but valid) navigation feed.
    let (status, series, _) = get(router(&env, None), "/opds/series").await;
    assert_eq!(status, StatusCode::OK);
    assert!(series.contains("<title>Series</title>"));
}

#[tokio::test]
async fn search_matches_and_excludes_fileless_books() {
    let (env, _) = setup_env();

    let (status, hit, _) = get(router(&env, None), "/opds/search?q=Dune").await;
    assert_eq!(status, StatusCode::OK);
    assert!(hit.contains("<title>Dune</title>"));

    // Neuromancer matches text but has no file, so it is not downloadable.
    let (_, miss, _) = get(router(&env, None), "/opds/search?q=Neuromancer").await;
    assert!(!miss.contains("<title>Neuromancer</title>"));

    // Empty query yields a valid, empty feed.
    let (status, empty, _) = get(router(&env, None), "/opds/search?q=").await;
    assert_eq!(status, StatusCode::OK);
    assert!(empty.contains("<feed"));
}

#[tokio::test]
async fn download_serves_associated_file_bytes() {
    let (env, file_id) = setup_env();
    let resp = router(&env, None)
        .oneshot(
            Request::builder()
                .uri(format!("/opds/download/{file_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/epub+zip"
    );
    let disposition = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains("dune.epub"));
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"epub-bytes");
}

#[tokio::test]
async fn download_unknown_file_is_404() {
    let (env, _) = setup_env();
    let unknown = uuid::Uuid::now_v7();
    let (status, _, _) = get(router(&env, None), &format!("/opds/download/{unknown}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn opensearch_document_is_served() {
    let (env, _) = setup_env();
    let (status, body, ct) = get(router(&env, None), "/opds/opensearch.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.unwrap().contains("opensearchdescription"));
    assert!(body.contains("template=\"/opds/search?q={searchTerms}\""));
}

fn auth_config() -> OpdsConfig {
    OpdsConfig {
        username: Some("reader".into()),
        password_hash: Some(OpdsConfig::hash_password("lanpass")),
    }
}

#[tokio::test]
async fn auth_required_when_configured() {
    let (env, _) = setup_env();
    let (status, _, _) = get(router(&env, Some(auth_config())), "/opds").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_rejects_wrong_password() {
    let (env, _) = setup_env();
    let creds = base64::engine::general_purpose::STANDARD.encode("reader:wrong");
    let resp = router(&env, Some(auth_config()))
        .oneshot(
            Request::builder()
                .uri("/opds")
                .header(header::AUTHORIZATION, format!("Basic {creds}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp.headers().contains_key(header::WWW_AUTHENTICATE));
}

#[tokio::test]
async fn auth_accepts_correct_credentials() {
    let (env, _) = setup_env();
    let creds = base64::engine::general_purpose::STANDARD.encode("reader:lanpass");
    let resp = router(&env, Some(auth_config()))
        .oneshot(
            Request::builder()
                .uri("/opds")
                .header(header::AUTHORIZATION, format!("Basic {creds}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
