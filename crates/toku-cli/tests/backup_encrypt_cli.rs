//! End-to-end CLI tests for offline, passphrase-encrypted backups (#204).
//!
//! Drives the real `toku` binary against throwaway data directories with **no
//! sync configured**, exercising the offline-first path: `export backup
//! --encrypt` seals with a passphrase-derived local key (KDF salt embedded in
//! the archive), and `import backup` restores it on a *fresh profile* using only
//! the passphrase. The passphrase is supplied non-interactively via
//! `TOKU_BACKUP_PASSPHRASE` so the tests need no TTY.

use std::path::Path;
use std::process::{Command, ExitStatus};

use toku_db::{BookRepository, Database};

const PASSPHRASE: &str = "correct horse battery staple";

/// Run `toku` against `data_dir`, optionally supplying a backup passphrase via
/// the environment. Returns the exit status (stdout/stderr inherited).
fn toku_with_passphrase(data_dir: &Path, args: &[&str], passphrase: Option<&str>) -> ExitStatus {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_toku"));
    cmd.arg("--data-dir").arg(data_dir).args(args);
    match passphrase {
        Some(p) => {
            cmd.env("TOKU_BACKUP_PASSPHRASE", p);
        }
        None => {
            cmd.env_remove("TOKU_BACKUP_PASSPHRASE");
        }
    }
    cmd.status().expect("run toku")
}

fn book_count(data_dir: &Path) -> usize {
    let db = Database::open(&data_dir.join("toku.db")).expect("open db");
    BookRepository::new(&db)
        .list_books()
        .expect("list books")
        .len()
}

#[test]
fn offline_passphrase_backup_roundtrips_on_a_fresh_profile() {
    let src = tempfile::tempdir().unwrap();
    assert!(
        toku_with_passphrase(
            src.path(),
            &["add", "--title", "Dune", "--author", "Frank Herbert"],
            None
        )
        .success()
    );

    let zip = src.path().join("backup.zip");
    let zip_str = zip.to_str().unwrap();

    // Seal with the passphrase (offline: no sync configured).
    assert!(
        toku_with_passphrase(
            src.path(),
            &["export", "backup", "--output", zip_str, "--encrypt"],
            Some(PASSPHRASE),
        )
        .success(),
        "offline --encrypt export must succeed with a passphrase"
    );
    assert!(zip.exists(), "backup archive must be written");

    // Restore on a brand-new profile using only the passphrase.
    let dst = tempfile::tempdir().unwrap();
    assert!(
        toku_with_passphrase(
            dst.path(),
            &["import", "backup", zip_str, "--replace"],
            Some(PASSPHRASE),
        )
        .success(),
        "restore on a fresh profile must succeed with the same passphrase"
    );
    assert_eq!(
        book_count(dst.path()),
        1,
        "the restored library must contain the book"
    );
}

#[test]
fn wrong_passphrase_restore_fails_cleanly() {
    let src = tempfile::tempdir().unwrap();
    assert!(
        toku_with_passphrase(
            src.path(),
            &["add", "--title", "Dune", "--author", "Frank Herbert"],
            None
        )
        .success()
    );

    let zip = src.path().join("backup.zip");
    let zip_str = zip.to_str().unwrap();
    assert!(
        toku_with_passphrase(
            src.path(),
            &["export", "backup", "--output", zip_str, "--encrypt"],
            Some(PASSPHRASE),
        )
        .success()
    );

    let dst = tempfile::tempdir().unwrap();
    let status = toku_with_passphrase(
        dst.path(),
        &["import", "backup", zip_str, "--replace"],
        Some("the wrong passphrase"),
    );
    assert!(
        !status.success(),
        "a wrong passphrase must fail the restore, not silently succeed"
    );
}

#[test]
fn plaintext_backup_still_works_without_a_passphrase() {
    let src = tempfile::tempdir().unwrap();
    assert!(
        toku_with_passphrase(
            src.path(),
            &["add", "--title", "Dune", "--author", "Frank Herbert"],
            None
        )
        .success()
    );

    let zip = src.path().join("plain.zip");
    let zip_str = zip.to_str().unwrap();
    // No --encrypt, no passphrase: the default plaintext artifact is unchanged.
    assert!(
        toku_with_passphrase(src.path(), &["export", "backup", "--output", zip_str], None)
            .success()
    );

    let dst = tempfile::tempdir().unwrap();
    assert!(
        toku_with_passphrase(
            dst.path(),
            &["import", "backup", zip_str, "--replace"],
            None
        )
        .success()
    );
    assert_eq!(book_count(dst.path()), 1);
}
