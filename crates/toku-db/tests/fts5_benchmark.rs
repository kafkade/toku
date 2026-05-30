//! Spike #18: FTS5 search performance at scale.
//!
//! Creates a synthetic database with 10,000 books and measures
//! FTS5 query latency for various query patterns.
//!
//! Run with: cargo test -p toku-db --test fts5_benchmark -- --nocapture --ignored

use std::time::Instant;

use toku_core::Book;
use toku_db::{BookRepository, Database};

const BOOK_COUNT: usize = 10_000;

// Word pools for generating realistic book metadata.
const TITLE_WORDS: &[&str] = &[
    "The",
    "A",
    "Dark",
    "Light",
    "Last",
    "First",
    "Secret",
    "Lost",
    "Shadow",
    "Silent",
    "Fire",
    "Ice",
    "Storm",
    "Wind",
    "Night",
    "Dawn",
    "Crown",
    "Throne",
    "Dragon",
    "Wolf",
    "Garden",
    "Tower",
    "Bridge",
    "River",
    "Mountain",
    "Ocean",
    "Star",
    "Moon",
    "Sun",
    "Sky",
    "Empire",
    "Kingdom",
    "World",
    "City",
    "Forest",
    "Desert",
    "Island",
    "Shore",
    "Gate",
    "Dream",
    "Memory",
    "Echo",
    "Whisper",
    "Song",
    "Dance",
    "War",
    "Peace",
    "Heart",
    "Soul",
    "Blood",
    "Bone",
    "Glass",
    "Iron",
    "Silver",
    "Gold",
    "Crimson",
    "Azure",
    "Emerald",
    "Fallen",
    "Rising",
    "Broken",
    "Eternal",
    "Ancient",
    "Final",
    "Infinite",
    "Forgotten",
    "Hidden",
    "Sacred",
    "Cursed",
    "Blessed",
    "Burning",
    "Frozen",
    "Sleeping",
    "Waking",
];

const DESC_PHRASES: &[&str] = &[
    "A sweeping epic of love and war in a distant galaxy",
    "In the ruins of civilization, one woman fights to survive",
    "A detective uncovers dark secrets beneath the city streets",
    "Two strangers meet on a train and their lives change forever",
    "An ancient prophecy awakens as the world teeters on the brink",
    "A young scientist discovers a formula that could save humanity",
    "In a world where magic is forbidden, one child dares to dream",
    "The last dragon rider must find the courage to face the darkness",
    "A family saga spanning three generations and two continents",
    "A thriller set in the corridors of power where no one can be trusted",
    "An enchanting tale of friendship set in a small coastal town",
    "A philosophical journey through the nature of consciousness and reality",
    "When the stars align, the portal between worlds opens once more",
    "A gripping mystery that keeps you guessing until the final page",
    "In the aftermath of the great plague, survivors build a new society",
    "A coming-of-age story set against the backdrop of revolution",
    "The biography of an extraordinary inventor who changed the world",
    "A poetic meditation on time, memory, and the landscapes of the mind",
    "An alternate history where the Roman Empire never fell",
    "Deep beneath the ocean, explorers find something ancient and alive",
];

const GENRES: &[&str] = &[
    "science fiction",
    "fantasy",
    "mystery",
    "thriller",
    "romance",
    "literary fiction",
    "horror",
    "historical fiction",
    "non-fiction",
    "biography",
    "philosophy",
    "poetry",
    "adventure",
    "dystopian",
    "cyberpunk",
    "steampunk",
    "magical realism",
    "young adult",
];

fn generate_title(i: usize) -> String {
    let w1 = TITLE_WORDS[i % TITLE_WORDS.len()];
    let w2 = TITLE_WORDS[(i * 7 + 13) % TITLE_WORDS.len()];
    let w3 = TITLE_WORDS[(i * 3 + 29) % TITLE_WORDS.len()];
    if i.is_multiple_of(3) {
        format!("{w1} {w2}")
    } else if i % 3 == 1 {
        format!("The {w1} of {w2}")
    } else {
        format!("{w1} {w2}: {w3}")
    }
}

fn generate_description(i: usize) -> String {
    let base = DESC_PHRASES[i % DESC_PHRASES.len()];
    let genre = GENRES[i % GENRES.len()];
    format!("{base}. A masterwork of {genre} that explores the depths of human experience.")
}

fn setup_db() -> (Database, Vec<String>) {
    let db = Database::open_in_memory().unwrap();
    let repo = BookRepository::new(&db);
    let mut titles = Vec::with_capacity(BOOK_COUNT);

    let start = Instant::now();

    for i in 0..BOOK_COUNT {
        let title = generate_title(i);
        let mut book = Book::new(&title);
        book.description = Some(generate_description(i));
        book.page_count = Some(200 + (i % 800) as i32);
        repo.create_book(&book).unwrap();
        titles.push(title);
    }

    let elapsed = start.elapsed();
    eprintln!(
        "\n📦 Inserted {BOOK_COUNT} books in {:.1}ms ({:.0} books/sec)",
        elapsed.as_millis(),
        BOOK_COUNT as f64 / elapsed.as_secs_f64()
    );

    (db, titles)
}

fn bench_query(repo: &BookRepository, label: &str, query: &str) -> std::time::Duration {
    // Warm up
    let _ = repo.search_books(query);

    // Measure 10 iterations
    let start = Instant::now();
    let iterations = 10;
    let mut result_count = 0;

    for _ in 0..iterations {
        let results = repo.search_books(query).unwrap();
        result_count = results.len();
    }

    let elapsed = start.elapsed();
    let per_query = elapsed / iterations;

    eprintln!("  {label:<30} {per_query:>8.1?}  ({result_count} results)",);

    per_query
}

#[test]
#[ignore] // Run explicitly: cargo test -p toku-db --test fts5_benchmark -- --nocapture --ignored
fn fts5_performance_spike() {
    eprintln!("\n🔍 FTS5 Performance Spike — {BOOK_COUNT} books\n");

    let (db, _titles) = setup_db();
    let repo = BookRepository::new(&db);

    // Verify count
    let count = repo.list_books().unwrap().len();
    assert_eq!(count, BOOK_COUNT);

    eprintln!("\n📊 Query latency (avg of 10 runs):\n");

    // Single word queries
    let t1 = bench_query(&repo, "Single word (common)", "Dragon");
    let t2 = bench_query(&repo, "Single word (rare)", "cyberpunk");

    // Multi-word queries
    let t3 = bench_query(&repo, "Multi-word (AND)", "Dark Shadow");
    let t4 = bench_query(&repo, "Multi-word (phrase)", "\"love and war\"");

    // Prefix queries
    let t5 = bench_query(&repo, "Prefix", "Drag*");
    let t6 = bench_query(&repo, "Prefix (short)", "Th*");

    // Broad queries (many results) — these hit all 10k rows.
    // In production, results would be paginated with LIMIT.
    // We measure but don't hold to the 100ms target for exhaustive scans.
    let t7 = bench_query(&repo, "Broad (The)", "The");
    let t8 = bench_query(&repo, "Broad (human)", "human");

    // No results
    let t9 = bench_query(&repo, "No results", "xyzzyplugh");

    // Assert targeted queries are under 100ms.
    // Broad queries that return ALL rows are excluded — they're
    // an allocation/serialization cost, not an FTS5 cost.
    let max_target = std::time::Duration::from_millis(100);
    eprintln!("\n✅ Acceptance criteria: targeted queries < 100ms\n");

    let targeted = [t1, t2, t3, t4, t5, t9];
    let broad = [t6, t7, t8];
    let max_targeted = targeted.iter().max().unwrap();
    let avg_targeted: std::time::Duration =
        targeted.iter().sum::<std::time::Duration>() / targeted.len() as u32;
    let max_broad = broad.iter().max().unwrap();

    eprintln!("  Max targeted query: {max_targeted:.1?}");
    eprintln!("  Avg targeted query: {avg_targeted:.1?}");
    eprintln!("  Max broad query:    {max_broad:.1?} (all {BOOK_COUNT} rows — not gated)");
    eprintln!("  Target:             {max_target:?}");

    assert!(
        *max_targeted < max_target,
        "FAILED: max targeted query time {max_targeted:?} exceeds 100ms target"
    );

    // Measure list_books performance (non-FTS)
    eprintln!("\n📊 Non-FTS operations:\n");

    let start = Instant::now();
    let _all = repo.list_books().unwrap();
    let list_time = start.elapsed();
    eprintln!("  list_books ({BOOK_COUNT}):        {list_time:.1?}");

    // Measure database size
    let page_count: i64 = db
        .conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let page_size: i64 = db
        .conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap();
    let db_size_mb = (page_count * page_size) as f64 / (1024.0 * 1024.0);
    eprintln!("\n💾 Database size: {db_size_mb:.1} MB for {BOOK_COUNT} books (no covers)");

    eprintln!("\n✅ Spike complete — FTS5 performance validated\n");
}
