//! Backup container round-trip tests: versioned JSON-in-ZIP with
//! content-addressed binaries, optional encryption, and v1 rejection
//! (ADR-012, issue #200). Runs under `cargo test --workspace`.

use std::fs;
use std::io::Write;
use std::path::Path;

use toku_core::SyncKey;
use toku_db::{Database, LibraryIo, RestoreMode};
use toku_export::{export_backup, export_backup_with_kdf, import_backup, read_backup_manifest};

const COVER_HASH: &str = "covhash123";
const COVER_BYTES: &[u8] = b"\xFF\xD8\xFFfake-jpeg-cover-bytes";
const EBOOK_BYTES: &[u8] = b"PK\x03\x04fake-epub-payload-bytes";
const EBOOK_CHECKSUM: &str = "sha-ebook-999";

/// Seed a book plus a cover binary and an ebook binary on disk under `data_dir`.
/// Returns the absolute ebook path recorded in the `files` table.
fn seed_with_binaries(db: &Database, data_dir: &Path) -> std::path::PathBuf {
    let covers_dir = data_dir.join("covers");
    fs::create_dir_all(&covers_dir).unwrap();
    fs::write(covers_dir.join(format!("{COVER_HASH}.jpg")), COVER_BYTES).unwrap();

    let files_dir = data_dir.join("files");
    fs::create_dir_all(&files_dir).unwrap();
    let ebook_path = files_dir.join("dune.epub");
    fs::write(&ebook_path, EBOOK_BYTES).unwrap();

    let c = &db.conn;
    c.execute(
        "INSERT INTO books
            (id, title, subtitle, description, page_count, format, status, rating,
             cover_hash, created_at, updated_at, search_text)
         VALUES
            ('b1', 'Dune', 'Book One', 'Desert.', 412, 'ebook', 'read', 9,
             ?1, '2021-01-01T00:00:00Z', '2021-01-01T00:00:00Z', 'Dune')",
        rusqlite::params![COVER_HASH],
    )
    .unwrap();
    c.execute(
        "INSERT INTO authors (id, name, sort_name) VALUES ('a1', 'Frank Herbert', 'Herbert, Frank')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO book_authors (book_id, author_id, role, position) VALUES ('b1', 'a1', 'author', 0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO isbns (isbn, book_id) VALUES ('9780441172719', 'b1')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO files (id, book_id, path, format, size_bytes, checksum, source, source_ref, created_at, updated_at)
         VALUES ('f1', 'b1', ?1, 'epub', ?2, ?3, 'user', NULL, '2021-01-01T00:00:00Z', '2021-01-01T00:00:00Z')",
        rusqlite::params![ebook_path.to_string_lossy(), EBOOK_BYTES.len() as i64, EBOOK_CHECKSUM],
    )
    .unwrap();

    ebook_path
}

#[test]
fn zip_roundtrip_restores_data_and_binaries() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let src = Database::open_in_memory().unwrap();
    let ebook_path = seed_with_binaries(&src, &src_dir);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    let zip_path = tmp.path().join("backup.zip");
    export_backup(&src, &src_dir, &zip_path, None).unwrap();
    assert!(zip_path.exists());

    // Manifest reports plaintext v2 with the expected counts.
    let manifest = read_backup_manifest(&zip_path).unwrap();
    assert_eq!(manifest.format_version, 2);
    assert!(!manifest.encrypted);
    assert_eq!(manifest.counts.books, 1);
    assert_eq!(manifest.counts.covers, 1);
    assert_eq!(manifest.counts.files, 1);

    // Remove the on-disk binary so extraction has to recreate it.
    fs::remove_file(&ebook_path).unwrap();

    // Restore into a fresh DB + fresh data_dir.
    let dst_dir = tmp.path().join("dst");
    fs::create_dir_all(&dst_dir).unwrap();
    let dst = Database::open_in_memory().unwrap();
    let result = import_backup(&zip_path, &dst, &dst_dir, RestoreMode::Replace, None).unwrap();
    assert_eq!(result.books_inserted, 1);

    // Structural equality of the whole library.
    let reexported = LibraryIo::new(&dst).export_library().unwrap();
    assert_eq!(exported, reexported);

    // Cover extracted content-addressed under the destination data dir.
    let cover_dest = dst_dir.join("covers").join(format!("{COVER_HASH}.jpg"));
    assert!(cover_dest.exists(), "cover must be extracted");
    assert_eq!(fs::read(&cover_dest).unwrap(), COVER_BYTES);

    // Ebook reconstructed byte-for-byte at its recorded path.
    assert!(ebook_path.exists(), "ebook must be re-extracted");
    assert_eq!(fs::read(&ebook_path).unwrap(), EBOOK_BYTES);
}

#[test]
fn encrypted_roundtrip_seals_library_json() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let src = Database::open_in_memory().unwrap();
    seed_with_binaries(&src, &src_dir);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    let salt = SyncKey::generate_salt().unwrap();
    let key = SyncKey::derive("correct horse battery staple", &salt).unwrap();

    let zip_path = tmp.path().join("secret.zip");
    export_backup(&src, &src_dir, &zip_path, Some(&key)).unwrap();

    let manifest = read_backup_manifest(&zip_path).unwrap();
    assert!(manifest.encrypted, "manifest must flag encryption");
    assert!(
        manifest.envelope.is_some(),
        "sealed payload must ride in the manifest"
    );

    // library.json must NOT be present in cleartext inside the archive.
    let raw = fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(raw).unwrap();
    assert!(
        archive.by_name("library.json").is_err(),
        "encrypted archive must not contain a plaintext library.json"
    );
    drop(archive);

    // Decrypt with the key → identical data.
    let dst_dir = tmp.path().join("dst");
    fs::create_dir_all(&dst_dir).unwrap();
    let dst = Database::open_in_memory().unwrap();
    import_backup(&zip_path, &dst, &dst_dir, RestoreMode::Replace, Some(&key)).unwrap();
    let reexported = LibraryIo::new(&dst).export_library().unwrap();
    assert_eq!(exported, reexported);
}

#[test]
fn encrypted_backup_without_key_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let src = Database::open_in_memory().unwrap();
    seed_with_binaries(&src, &src_dir);

    let salt = SyncKey::generate_salt().unwrap();
    let key = SyncKey::derive("passphrase", &salt).unwrap();
    let zip_path = tmp.path().join("secret.zip");
    export_backup(&src, &src_dir, &zip_path, Some(&key)).unwrap();

    let dst = Database::open_in_memory().unwrap();
    let err = import_backup(&zip_path, &dst, tmp.path(), RestoreMode::Replace, None)
        .expect_err("must refuse to restore an encrypted backup without a key");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("encrypt") || msg.contains("key"),
        "actionable error: {msg}"
    );
}

#[test]
fn passphrase_backup_restores_on_a_fresh_profile() {
    use toku_core::backup_schema::BackupKdf;

    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let src = Database::open_in_memory().unwrap();
    seed_with_binaries(&src, &src_dir);
    let exported = LibraryIo::new(&src).export_library().unwrap();

    // Seal with a passphrase-derived local key; the KDF descriptor is embedded
    // in the manifest so the archive is self-describing.
    let passphrase = "correct horse battery staple";
    let kdf = BackupKdf::generate().unwrap();
    let key = kdf.derive_key(passphrase).unwrap();
    let zip_path = tmp.path().join("passphrase.zip");
    export_backup_with_kdf(&src, &src_dir, &zip_path, &key, kdf).unwrap();

    // Manifest is self-describing: encrypted, envelope present, kdf present, and
    // no plaintext library.json inside the archive.
    let manifest = read_backup_manifest(&zip_path).unwrap();
    assert!(manifest.encrypted, "manifest must flag encryption");
    assert!(
        manifest.envelope.is_some(),
        "sealed payload rides in manifest"
    );
    let embedded = manifest
        .kdf
        .as_ref()
        .expect("kdf descriptor must travel in the manifest for portability");
    assert_eq!(embedded.algorithm, "argon2id");
    let raw = fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(raw).unwrap();
    assert!(
        archive.by_name("library.json").is_err(),
        "encrypted archive must not contain a plaintext library.json"
    );
    drop(archive);

    // Restore on a FRESH profile using only the passphrase + embedded salt — no
    // local config, no sync enrollment.
    let dst_dir = tmp.path().join("dst");
    fs::create_dir_all(&dst_dir).unwrap();
    let dst = Database::open_in_memory().unwrap();
    let rederived = embedded.derive_key(passphrase).unwrap();
    import_backup(
        &zip_path,
        &dst,
        &dst_dir,
        RestoreMode::Replace,
        Some(&rederived),
    )
    .unwrap();
    let reexported = LibraryIo::new(&dst).export_library().unwrap();
    assert_eq!(exported, reexported);
}

#[test]
fn passphrase_backup_wrong_passphrase_fails_cleanly() {
    use toku_core::backup_schema::BackupKdf;

    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let src = Database::open_in_memory().unwrap();
    seed_with_binaries(&src, &src_dir);

    let kdf = BackupKdf::generate().unwrap();
    let key = kdf.derive_key("right passphrase").unwrap();
    let zip_path = tmp.path().join("passphrase.zip");
    export_backup_with_kdf(&src, &src_dir, &zip_path, &key, kdf).unwrap();

    // A wrong passphrase derives a different key; decryption must fail cleanly
    // (AEAD tag mismatch) rather than panic or return garbage.
    let manifest = read_backup_manifest(&zip_path).unwrap();
    let embedded = manifest.kdf.expect("kdf descriptor present");
    let wrong = embedded.derive_key("wrong passphrase").unwrap();

    let dst = Database::open_in_memory().unwrap();
    let err = import_backup(
        &zip_path,
        &dst,
        tmp.path(),
        RestoreMode::Replace,
        Some(&wrong),
    )
    .expect_err("wrong passphrase must not decrypt");
    assert!(
        matches!(err, toku_export::ExportError::Crypto(_)),
        "expected a crypto error, got: {err:?}"
    );
}

#[test]
fn v1_flat_manifest_is_rejected_with_actionable_error() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("old.zip");

    // Hand-build a superseded v1 archive: a manifest with "version" (not
    // "format_version").
    let file = fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions = Default::default();
    zip.start_file("manifest.json", opts).unwrap();
    zip.write_all(br#"{"version": 1, "created_at": "2020-01-01T00:00:00Z"}"#)
        .unwrap();
    zip.start_file("library.json", opts).unwrap();
    zip.write_all(b"{}").unwrap();
    zip.finish().unwrap();

    let dst = Database::open_in_memory().unwrap();
    let err = import_backup(&zip_path, &dst, tmp.path(), RestoreMode::Replace, None)
        .expect_err("v1 flat archive must be rejected");
    let msg = err.to_string();
    // The error must be actionable (mention re-exporting / superseded format).
    assert!(
        msg.to_lowercase().contains("supersed")
            || msg.to_lowercase().contains("export backup")
            || msg.to_lowercase().contains("version"),
        "error should guide the user: {msg}"
    );
}

#[test]
fn merge_import_is_idempotent_over_the_container() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let src = Database::open_in_memory().unwrap();
    seed_with_binaries(&src, &src_dir);

    let zip_path = tmp.path().join("backup.zip");
    export_backup(&src, &src_dir, &zip_path, None).unwrap();

    let dst_dir = tmp.path().join("dst");
    fs::create_dir_all(&dst_dir).unwrap();
    let dst = Database::open_in_memory().unwrap();
    import_backup(&zip_path, &dst, &dst_dir, RestoreMode::Merge, None).unwrap();
    let after_first = LibraryIo::new(&dst).export_library().unwrap();
    import_backup(&zip_path, &dst, &dst_dir, RestoreMode::Merge, None).unwrap();
    let after_second = LibraryIo::new(&dst).export_library().unwrap();

    assert_eq!(after_first, after_second);
}
