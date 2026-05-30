//! Integration tests for Open Library API validation (issue #15).
//!
//! All tests are `#[ignore]` because they hit real network endpoints.
//! Run with: `cargo test -p toku-meta -- --ignored`

use std::time::Instant;
use toku_meta::{fetch_by_isbn, fetch_cover, search_books};

// ── ISBN lookups ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn isbn_english_bestseller_dune() {
    let start = Instant::now();
    let book = fetch_by_isbn("9780441172719").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Authors:     {:?}", book.authors);
    println!("  Pages:       {:?}", book.page_count);
    println!("  Pub date:    {:?}", book.pub_date);
    println!("  Language:    {:?}", book.language);
    println!("  Publishers:  {:?}", book.publishers);
    println!("  ISBN-13:     {:?}", book.isbn_13);
    println!("  ISBN-10:     {:?}", book.isbn_10);
    println!("  OL ID:       {:?}", book.openlibrary_id);
    println!("  Cover ID:    {:?}", book.cover_id);
    println!("  Response:    {elapsed:?}");

    assert!(book.title.contains("Dune"));
    assert!(!book.authors.is_empty());
    assert!(book.page_count.is_some());
}

#[tokio::test]
#[ignore]
async fn isbn_english_bestseller_harry_potter() {
    let start = Instant::now();
    let book = fetch_by_isbn("9780590353427").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Authors:     {:?}", book.authors);
    println!("  Pages:       {:?}", book.page_count);
    println!("  Cover ID:    {:?}", book.cover_id);
    println!("  Response:    {elapsed:?}");

    assert!(book.title.contains("Harry Potter"));
    // Known gap: some OL editions lack author links at the edition level.
    // The work-level record has authors, but the edition may not.
    if book.authors.is_empty() {
        println!("  ⚠ AUTHORS MISSING — edition lacks author references");
    }
}

#[tokio::test]
#[ignore]
async fn isbn_non_english_spanish() {
    // Cien años de soledad — Spanish edition
    let start = Instant::now();
    let book = fetch_by_isbn("9788497592208").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Authors:     {:?}", book.authors);
    println!("  Language:    {:?}", book.language);
    println!("  Pages:       {:?}", book.page_count);
    println!("  Response:    {elapsed:?}");

    assert!(!book.title.is_empty());
    // Known gap: some OL editions have no author links (e.g., mass-market reprints).
    // The work record has authors, but the edition may not reference them.
    if book.authors.is_empty() {
        println!("  ⚠ AUTHORS MISSING — edition lacks author references");
    }
}

#[tokio::test]
#[ignore]
async fn isbn_non_english_japanese() {
    // Norwegian Wood (ノルウェイの森) — Japanese edition
    let start = Instant::now();
    let result = fetch_by_isbn("9784062749688").await;
    let elapsed = start.elapsed();

    match result {
        Ok(book) => {
            println!("  Title:       {}", book.title);
            println!("  Language:    {:?}", book.language);
            println!("  Response:    {elapsed:?}");
        }
        Err(e) => {
            println!("  NOT FOUND:   {e}");
            println!("  Response:    {elapsed:?}");
            println!("  (Japanese editions have poor coverage)");
        }
    }
}

#[tokio::test]
#[ignore]
async fn isbn_old_edition_pre_1970_mockingbird() {
    // To Kill a Mockingbird (1960, HarperCollins reprint)
    let start = Instant::now();
    let book = fetch_by_isbn("9780061120084").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Authors:     {:?}", book.authors);
    println!("  Pub date:    {:?}", book.pub_date);
    println!("  Pages:       {:?}", book.page_count);
    println!("  Response:    {elapsed:?}");

    assert!(book.title.contains("Mockingbird"));
}

#[tokio::test]
#[ignore]
async fn isbn_old_edition_pre_1970_1984() {
    // 1984 by George Orwell (Signet Classic)
    let start = Instant::now();
    let book = fetch_by_isbn("9780451524935").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Authors:     {:?}", book.authors);
    println!("  Pub date:    {:?}", book.pub_date);
    println!("  Response:    {elapsed:?}");

    // Known gap: Signet Classic edition returns title "Nineteen Eighty-Four"
    // and has no author links at the edition level.
    if book.authors.is_empty() {
        println!("  ⚠ AUTHORS MISSING — edition lacks author references");
    }
}

#[tokio::test]
#[ignore]
async fn isbn_self_published_the_martian() {
    // The Martian — originally self-published (Crown edition)
    let start = Instant::now();
    let book = fetch_by_isbn("9780553418026").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Authors:     {:?}", book.authors);
    println!("  Pub date:    {:?}", book.pub_date);
    println!("  Pages:       {:?}", book.page_count);
    println!("  Response:    {elapsed:?}");

    assert!(book.title.contains("Martian"));
}

#[tokio::test]
#[ignore]
async fn isbn_not_found() {
    // Bogus ISBN that should not exist
    let start = Instant::now();
    let result = fetch_by_isbn("9799999999999").await;
    let elapsed = start.elapsed();

    println!("  Result:      {result:?}");
    println!("  Response:    {elapsed:?}");

    assert!(result.is_err());
}

#[tokio::test]
#[ignore]
async fn isbn_no_pages_no_description() {
    // Some editions lack page count or description — test a niche edition
    // The Tao Te Ching — ancient text with many editions, some sparse
    let start = Instant::now();
    let book = fetch_by_isbn("9780679724346").await.unwrap();
    let elapsed = start.elapsed();

    println!("  Title:       {}", book.title);
    println!("  Pages:       {:?}", book.page_count);
    println!(
        "  Description: {:?}",
        book.description.as_ref().map(|d| &d[..d.len().min(80)])
    );
    println!("  Response:    {elapsed:?}");
}

// ── Search queries ──────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn search_by_title() {
    let start = Instant::now();
    let results = search_books("Dune Frank Herbert", 5).await.unwrap();
    let elapsed = start.elapsed();

    println!("  Results:     {}", results.len());
    for r in &results {
        println!(
            "    - {} by {:?} (editions: {}, isbn: {:?})",
            r.title, r.authors, r.edition_count, r.isbn
        );
    }
    println!("  Response:    {elapsed:?}");

    assert!(!results.is_empty());
    assert!(results.iter().any(|r| r.title.contains("Dune")));
}

#[tokio::test]
#[ignore]
async fn search_by_author_only() {
    let start = Instant::now();
    let results = search_books("author:Ursula K. Le Guin", 10).await.unwrap();
    let elapsed = start.elapsed();

    println!("  Results:     {}", results.len());
    for r in &results {
        println!("    - {} ({:?})", r.title, r.first_publish_year);
    }
    println!("  Response:    {elapsed:?}");

    assert!(!results.is_empty());
}

#[tokio::test]
#[ignore]
async fn search_non_english_title() {
    let start = Instant::now();
    let results = search_books("Cien años de soledad", 5).await.unwrap();
    let elapsed = start.elapsed();

    println!("  Results:     {}", results.len());
    for r in &results {
        println!(
            "    - {} by {:?} (languages: {:?})",
            r.title, r.authors, r.languages
        );
    }
    println!("  Response:    {elapsed:?}");

    assert!(!results.is_empty());
}

// ── Cover image API ─────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cover_available_dune() {
    let tmp = tempfile::TempDir::new().unwrap();
    let start = Instant::now();
    let result = fetch_cover("9780441172719", tmp.path()).await.unwrap();
    let elapsed = start.elapsed();

    println!("  Cover hash:  {result:?}");
    println!("  Response:    {elapsed:?}");

    assert!(result.is_some(), "Dune should have a cover");
    let hash = result.unwrap();
    let cover_path = tmp.path().join(format!("{hash}.jpg"));
    let size = std::fs::metadata(&cover_path).unwrap().len();
    println!("  File size:   {size} bytes");
    assert!(
        size > 5_000,
        "Cover should be a real image, not placeholder"
    );
}

#[tokio::test]
#[ignore]
async fn cover_not_available() {
    let tmp = tempfile::TempDir::new().unwrap();
    let start = Instant::now();
    // Use the bogus ISBN
    let result = fetch_cover("9799999999999", tmp.path()).await.unwrap();
    let elapsed = start.elapsed();

    println!("  Cover hash:  {result:?}");
    println!("  Response:    {elapsed:?}");

    assert!(result.is_none(), "Bogus ISBN should have no cover");
}

#[tokio::test]
#[ignore]
async fn cover_quality_harry_potter() {
    let tmp = tempfile::TempDir::new().unwrap();
    let start = Instant::now();
    let result = fetch_cover("9780590353427", tmp.path()).await.unwrap();
    let elapsed = start.elapsed();

    println!("  Cover hash:  {result:?}");
    println!("  Response:    {elapsed:?}");

    if let Some(hash) = result {
        let cover_path = tmp.path().join(format!("{hash}.jpg"));
        let size = std::fs::metadata(&cover_path).unwrap().len();
        println!("  File size:   {size} bytes (L-size)");
        // L-size covers should be reasonably large
        assert!(size > 10_000, "L-size cover should be high quality");
    } else {
        println!("  (No cover available)");
    }
}

// ── Response time benchmark ─────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn response_time_isbn_batch() {
    let isbns = [
        ("Dune", "9780441172719"),
        ("1984", "9780451524935"),
        ("Mockingbird", "9780061120084"),
        ("Harry Potter", "9780590353427"),
        ("The Martian", "9780553418026"),
    ];

    let mut times = Vec::new();
    for (label, isbn) in &isbns {
        let start = Instant::now();
        let result = fetch_by_isbn(isbn).await;
        let elapsed = start.elapsed();
        let status = if result.is_ok() { "OK" } else { "ERR" };
        println!("  {label:20} {status:3}  {elapsed:?}");
        times.push(elapsed.as_millis());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    let max = times.iter().max().unwrap();
    let min = times.iter().min().unwrap();
    println!("\n  Avg: {avg}ms  Min: {min}ms  Max: {max}ms");
    println!("  (5 sequential requests, no parallelism)");
}
