//! Multi-device sync integration tests (issue #78).
//!
//! Each test spins up a real in-process sync server and two or more simulated
//! devices, then drives them through the real sync client (`init`/`push`/
//! `pull`/`bootstrap`), the real merge engine, real HLC ordering, and real
//! AES-GCM crypto to assert that changes converge across devices.
//!
//! The harness (see `harness/mod.rs`) owns op *emission* only; everything below
//! it is production code.

mod harness;

use chrono::{DateTime, Utc};

use harness::{SimulatedDevice, TestServer};

fn at_ms(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).expect("valid timestamp")
}

/// Scenario 1 — Basic sync.
/// Device A adds a book and pushes; device B pulls and sees it.
#[test]
fn scenario_01_basic_sync() {
    let server = TestServer::start();
    let lib = "lib-01";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.add_book("The Left Hand of Darkness");
    let pushed = a.push();
    assert_eq!(pushed.accepted, 1);

    assert!(
        !b.book_exists(book),
        "B should not have the book before pull"
    );
    let pulled = b.pull();
    assert_eq!(pulled.pulled, 1, "B pulls exactly A's one op (not its own)");
    assert_eq!(pulled.applied, 1);

    assert!(b.book_exists(book), "B should have the book after pull");
    assert_eq!(
        b.book_title(book).as_deref(),
        Some("The Left Hand of Darkness")
    );
}

/// Scenario 2 — Concurrent edits to different fields.
/// A edits the title, B edits the rating; both edits survive on both devices.
#[test]
fn scenario_02_concurrent_different_fields() {
    let server = TestServer::start();
    let lib = "lib-02";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let mut b = SimulatedDevice::register(&server, lib, "device-b", None);

    // Establish a shared book on both devices.
    let book = a.add_book("Dune");
    a.push();
    b.pull();
    assert!(b.book_exists(book));

    // Concurrent edits to disjoint fields, before either has seen the other's.
    a.set_title(book, "Dune (Deluxe Edition)");
    b.set_rating(book, 9);

    a.push();
    b.push();
    a.pull();
    b.pull();

    for d in [&a, &b] {
        assert_eq!(
            d.book_title(book).as_deref(),
            Some("Dune (Deluxe Edition)"),
            "{} lost the title edit",
            d.name()
        );
        assert_eq!(
            d.book_rating(book),
            Some(9),
            "{} lost the rating edit",
            d.name()
        );
    }
}

/// Scenario 3 — Concurrent edits to the *same* field.
/// Both set the rating; the higher-HLC write wins (last-write-wins) and both
/// devices converge to it.
#[test]
fn scenario_03_concurrent_same_field_lww() {
    let server = TestServer::start();
    let lib = "lib-03";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let mut b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.add_book("Neuromancer");
    a.push();
    b.pull();

    // A writes rating 8 "earlier", B writes rating 9 "later" (higher HLC).
    a.set_rating_at(book, 8, at_ms(5_000));
    b.set_rating_at(book, 9, at_ms(6_000));

    a.push();
    b.push();
    a.pull();
    b.pull();

    assert_eq!(
        a.book_rating(book),
        Some(9),
        "A should converge to LWW value"
    );
    assert_eq!(
        b.book_rating(book),
        Some(9),
        "B should keep its newer value"
    );
}

/// Scenario 4 — Delete propagation.
/// A deletes a book; after B pulls, the book is gone on B.
#[test]
fn scenario_04_delete_propagation() {
    let server = TestServer::start();
    let lib = "lib-04";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.add_book("Snow Crash");
    a.push();
    b.pull();
    assert!(b.book_exists(book));

    a.delete_book(book);
    a.push();
    b.pull();

    assert!(!b.book_exists(book), "delete should propagate to B");
}

/// Scenario 5 — Delete versus concurrent edit.
/// A deletes while B edits (before seeing the delete). The delete wins on both
/// devices regardless of HLC ordering.
#[test]
fn scenario_05_delete_vs_edit() {
    let server = TestServer::start();
    let lib = "lib-05";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let mut b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.add_book("Hyperion");
    a.push();
    b.pull();

    // Concurrent: A deletes, B edits the rating without having pulled the delete.
    a.delete_book(book);
    b.set_rating(book, 10);

    a.push();
    b.push();
    a.pull();
    b.pull();

    assert!(!a.book_exists(book), "delete wins on A");
    assert!(!b.book_exists(book), "delete wins on B");
}

/// Scenario 6 — Offline edits then reconnect.
/// Both devices make multiple edits while "offline" (no sync), then reconnect
/// and exchange. All changes converge.
#[test]
fn scenario_06_offline_reconnect() {
    let server = TestServer::start();
    let lib = "lib-06";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let mut b = SimulatedDevice::register(&server, lib, "device-b", None);

    // Shared starting point.
    let shared = a.add_book("Foundation");
    a.push();
    b.pull();

    // Both go offline and make several independent edits.
    let a_only = a.add_book("Project Hail Mary");
    a.set_title(shared, "Foundation (Annotated)");

    let b_only = b.add_book("The Martian");
    b.set_rating(shared, 7);

    // Reconnect: exchange in both directions.
    a.push();
    b.push();
    a.pull();
    b.pull();

    for d in [&a, &b] {
        assert!(
            d.book_exists(a_only),
            "{} missing A's offline book",
            d.name()
        );
        assert!(
            d.book_exists(b_only),
            "{} missing B's offline book",
            d.name()
        );
        assert_eq!(
            d.book_title(shared).as_deref(),
            Some("Foundation (Annotated)"),
            "{} lost the shared title edit",
            d.name()
        );
        assert_eq!(
            d.book_rating(shared),
            Some(7),
            "{} lost the shared rating edit",
            d.name()
        );
    }
}

/// Scenario 7 — New-device bootstrap.
/// A populates a library and pushes. A freshly-registered device C bootstraps
/// and ends up with the full library.
///
/// Without op-log compaction the server still holds the full history, so
/// `bootstrap` converges via a full op-log pull (no snapshot). Here
/// `snapshot_applied` is expected to be false.
#[test]
fn scenario_07_new_device_bootstrap() {
    let server = TestServer::start();
    let lib = "lib-07";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let book1 = a.add_book("Children of Time");
    let book2 = a.add_book("Blindsight");
    a.set_rating(book2, 10);
    a.push();

    // New device joins and bootstraps.
    let c = SimulatedDevice::register(&server, lib, "device-c", None);
    let outcome = c.bootstrap();

    assert!(
        !outcome.snapshot_applied,
        "no compaction has run, so bootstrap should fall back to op-log pull"
    );
    assert!(c.book_exists(book1));
    assert!(c.book_exists(book2));
    assert_eq!(c.book_rating(book2), Some(10));
    assert_eq!(c.book_count(), 2);
}

/// Scenario 8 — Encryption round-trip.
/// Two devices share a passphrase. A pushes an (encrypted) op; B pulls,
/// decrypts, and recovers the plaintext.
#[test]
fn scenario_08_encryption_round_trip() {
    let server = TestServer::start();
    let lib = "lib-08";
    let pass = Some("correct horse battery staple");

    let mut a = SimulatedDevice::register(&server, lib, "device-a", pass);
    let b = SimulatedDevice::register(&server, lib, "device-b", pass);

    let book = a.add_book("Cryptonomicon");
    a.set_rating(book, 8);
    a.push();

    b.pull();

    assert_eq!(b.book_title(book).as_deref(), Some("Cryptonomicon"));
    assert_eq!(b.book_rating(book), Some(8));
}

/// Scenario 9 — Idempotent push.
/// Re-sending the exact same ops is deduplicated by the server (by op id) and
/// never corrupts state.
#[test]
fn scenario_09_idempotent_push() {
    let server = TestServer::start();
    let lib = "lib-09";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.add_book("A Fire Upon the Deep");
    let first = a.push();
    assert_eq!(first.accepted, 1);
    assert_eq!(first.duplicates, 0);

    // Re-send the very same op.
    a.force_repush();
    let second = a.push();
    assert_eq!(second.accepted, 0, "no new ops should be accepted");
    assert_eq!(second.duplicates, 1, "server should dedup the resent op");

    // No corruption: B sees exactly one book.
    b.pull();
    assert!(b.book_exists(book));
    assert_eq!(b.book_count(), 1);
}

/// Scenario 10 — Network-failure recovery (exactly-once).
/// A push reaches the server but the client loses the success acknowledgement
/// (simulated via `force_repush`). On retry the server dedups, so the change
/// is applied exactly once and the client ends up fully in sync.
#[test]
fn scenario_10_network_failure_recovery() {
    let server = TestServer::start();
    let lib = "lib-10";

    let mut a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.add_book("Anathem");

    // First attempt: ops reach the server, but the client "crashes" before
    // recording success — model this by clearing the pushed marker afterwards.
    a.push();
    a.force_repush();
    assert_eq!(a.pending_ops(), 1, "op looks unpushed after the lost ack");

    // Retry: server dedups, client records success.
    let retry = a.push();
    assert_eq!(retry.duplicates, 1);
    assert_eq!(retry.accepted, 0);
    assert_eq!(
        a.pending_ops(),
        0,
        "retry clears the pending op exactly once"
    );

    // Exactly-once on the peer.
    b.pull();
    assert!(b.book_exists(book));
    assert_eq!(b.book_count(), 1);
}
