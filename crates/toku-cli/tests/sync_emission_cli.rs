//! End-to-end CLI tests that drive the real `toku` binary and assert that
//! ordinary commands emit sync ops (issue #194).
//!
//! These run the compiled binary via `CARGO_BIN_EXE_toku`, pointing it at a
//! throwaway data directory. A device identity is seeded directly (equivalent
//! to having run `toku sync init`) so the offline-first guard is satisfied and
//! ops are emitted; the mutations themselves go through the real command path.

use std::path::Path;
use std::process::Command;

use toku_db::{Database, SyncRepository};

/// Seed a device identity in the CLI's database so op emission is enabled.
fn seed_device(data_dir: &Path) {
    let db = Database::open(&data_dir.join("toku.db")).expect("open db");
    SyncRepository::new(&db)
        .get_or_create_device("cli-test")
        .expect("seed device");
}

fn count_ops(data_dir: &Path, entity_type: &str, op_type: &str) -> i64 {
    let db = Database::open(&data_dir.join("toku.db")).expect("open db");
    db.conn
        .query_row(
            "SELECT COUNT(*) FROM sync_ops WHERE entity_type = ?1 AND op_type = ?2",
            [entity_type, op_type],
            |r| r.get(0),
        )
        .expect("count ops")
}

fn total_ops(data_dir: &Path) -> i64 {
    let db = Database::open(&data_dir.join("toku.db")).expect("open db");
    db.conn
        .query_row("SELECT COUNT(*) FROM sync_ops", [], |r| r.get(0))
        .expect("count ops")
}

fn toku(data_dir: &Path, args: &[&str]) {
    let status = Command::new(env!("CARGO_BIN_EXE_toku"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .status()
        .expect("run toku");
    assert!(status.success(), "toku {args:?} failed: {status}");
}

#[test]
fn add_command_emits_book_create_op() {
    let dir = tempfile::tempdir().unwrap();
    seed_device(dir.path());

    toku(
        dir.path(),
        &["add", "--title", "Dune", "--author", "Frank Herbert"],
    );

    assert_eq!(
        count_ops(dir.path(), "book", "create"),
        1,
        "a real `toku add` must stage exactly one Book Create op"
    );
}

#[test]
fn add_without_device_emits_no_ops() {
    // No seeded device — offline-first: the write must still work, but no ops.
    let dir = tempfile::tempdir().unwrap();

    toku(
        dir.path(),
        &["add", "--title", "Dune", "--author", "Frank Herbert"],
    );

    assert_eq!(
        total_ops(dir.path()),
        0,
        "without a device identity no ops may be staged"
    );
}

#[test]
fn tag_command_emits_tag_op() {
    let dir = tempfile::tempdir().unwrap();
    seed_device(dir.path());

    toku(
        dir.path(),
        &[
            "add",
            "--title",
            "Neuromancer",
            "--author",
            "William Gibson",
        ],
    );
    toku(dir.path(), &["tag", "add", "cyberpunk", "Neuromancer"]);

    assert_eq!(
        count_ops(dir.path(), "tag", "create"),
        1,
        "a real `toku tag add` must stage a Tag Create op"
    );
}

/// Run `toku` capturing stdout+status (for arg-parsing/help smoke tests).
fn toku_output(data_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_toku"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run toku")
}

#[test]
fn sync_bootstrap_command_is_wired_with_reset_cursor_flag() {
    // The new recovery verb (#199, ADR-013 D3) must be a real subcommand that
    // parses `--reset-cursor`. `--help` exercises the clap surface without
    // needing a configured sync server.
    let dir = tempfile::tempdir().unwrap();
    let out = toku_output(dir.path(), &["sync", "bootstrap", "--help"]);
    assert!(
        out.status.success(),
        "`toku sync bootstrap --help` should succeed"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--reset-cursor"),
        "bootstrap help must document --reset-cursor; got:\n{stdout}"
    );
}

#[test]
fn sync_bootstrap_without_config_errors_gracefully() {
    // With no sync configured, bootstrap must fail with a clean error (non-zero
    // exit), never panic — no `unwrap()` in the user-facing path.
    let dir = tempfile::tempdir().unwrap();
    let out = toku_output(dir.path(), &["sync", "bootstrap"]);
    assert!(
        !out.status.success(),
        "bootstrap without sync configured should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "bootstrap must not panic; stderr:\n{stderr}"
    );
}
