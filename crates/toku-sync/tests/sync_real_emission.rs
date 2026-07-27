//! Multi-device round-trip tests driven by **real frontend commands**.
//!
//! Unlike `sync_multi_device.rs` (which hand-builds ops to pin explicit HLC
//! times for deterministic LWW), these tests mutate through the product's
//! `BookRepository` — the exact code path the CLI and FFI use. They assert that
//! ordinary edits (add book, set status/rating, delete, log session/progress,
//! add tag) emit ops and round-trip to a second device. This is the acceptance
//! criterion for issue #194: real commands produce ops that propagate.

mod harness;

use harness::{SimulatedDevice, TestServer};
use toku_core::ReadingStatus;

/// Adding a book through the real repository emits exactly one op that
/// propagates to a second device.
#[test]
fn real_add_book_round_trips() {
    let server = TestServer::start();
    let lib = "real-01";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("Piranesi");
    assert!(
        a.book_exists(book),
        "A has the book immediately after the write"
    );
    assert_eq!(a.pending_ops(), 1, "exactly one op staged by create_book");

    assert_eq!(a.push().accepted, 1);
    let pulled = b.pull();
    assert_eq!(pulled.applied, 1);

    assert!(b.book_exists(book));
    assert_eq!(b.book_title(book).as_deref(), Some("Piranesi"));
}

/// Status and rating updates round-trip and materialize on the peer.
#[test]
fn real_status_and_rating_round_trip() {
    let server = TestServer::start();
    let lib = "real-02";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("Dune");
    a.repo_set_status(book, ReadingStatus::Reading);
    a.repo_set_rating(book, 9);
    assert_eq!(a.pending_ops(), 3, "create + status + rating");

    a.push();
    b.pull();

    assert_eq!(b.book_status(book).as_deref(), Some("reading"));
    assert_eq!(b.book_rating(book), Some(9));
}

/// A delete performed via the real repository tombstones the book on the peer.
#[test]
fn real_delete_round_trips() {
    let server = TestServer::start();
    let lib = "real-03";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("Ephemeral");
    a.push();
    b.pull();
    assert!(b.book_exists(book));

    a.repo_delete_book(book);
    a.push();
    b.pull();

    assert!(!b.book_exists(book), "delete op must propagate to B");
}

/// A reading session logged through the real repository round-trips.
#[test]
fn real_session_round_trips() {
    let server = TestServer::start();
    let lib = "real-04";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("Hyperion");
    a.repo_log_session(book);

    a.push();
    b.pull();

    assert!(b.book_exists(book));
    assert_eq!(b.session_count(book), 1, "session op must propagate to B");
}

/// A progress entry logged through the real repository round-trips.
#[test]
fn real_progress_round_trips() {
    let server = TestServer::start();
    let lib = "real-05";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("Cryptonomicon");
    a.repo_log_progress(book, 250);

    a.push();
    b.pull();

    assert_eq!(
        b.latest_progress(book),
        Some(250),
        "progress must propagate to B"
    );
}

/// A tag added through the real repository round-trips.
#[test]
fn real_tag_round_trips() {
    let server = TestServer::start();
    let lib = "real-06";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("Neuromancer");
    a.repo_add_tag(book, "cyberpunk");

    a.push();
    b.pull();

    assert!(b.book_exists(book));
    assert!(b.has_tag(book, "cyberpunk"), "tag op must propagate to B");
}

/// Everything together: a full edit session on A converges on B in one sync.
#[test]
fn real_full_edit_session_converges() {
    let server = TestServer::start();
    let lib = "real-07";

    let a = SimulatedDevice::register(&server, lib, "device-a", None);
    let b = SimulatedDevice::register(&server, lib, "device-b", None);

    let book = a.repo_add_book("The Dispossessed");
    a.repo_set_status(book, ReadingStatus::Reading);
    a.repo_log_session(book);
    a.repo_log_progress(book, 100);
    a.repo_add_tag(book, "utopia");
    a.repo_set_rating(book, 10);

    a.push();
    b.pull();

    assert_eq!(b.book_title(book).as_deref(), Some("The Dispossessed"));
    assert_eq!(b.book_status(book).as_deref(), Some("reading"));
    assert_eq!(b.book_rating(book), Some(10));
    assert_eq!(b.session_count(book), 1);
    assert_eq!(b.latest_progress(book), Some(100));
    assert!(b.has_tag(book, "utopia"));
}
