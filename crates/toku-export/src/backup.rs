//! Canonical, versioned, lossless backup container (ADR-012).
//!
//! A backup is a ZIP holding:
//! - `manifest.json` — [`BackupManifest`] (`format_version`, `created_at`,
//!   entity counts, `encrypted` flag + AEAD envelope when sealed).
//! - `library.json` — the full [`LibraryData`] domain schema (omitted when the
//!   backup is encrypted; the sealed payload then lives in the manifest
//!   envelope).
//! - `covers/<sha256>.jpg` — cover images, content-addressed by `cover_hash`.
//! - `files/<checksum>.<ext>` — ebook files, content-addressed by
//!   `files.checksum`.
//!
//! Export and restore both work fully offline. Encryption is an optional outer
//! wrapper over `library.json` using the snapshot AEAD path; the unencrypted
//! plaintext backup is the default artifact (ADR-012 D4).

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;

use toku_core::backup_schema::{BACKUP_FORMAT_VERSION, BackupManifest, LibraryData};
use toku_core::crypto::{EncryptedEnvelope, SyncKey, decrypt_snapshot, encrypt_snapshot};
use toku_db::{Database, LibraryIo, RestoreMode, RestoreResult};

use crate::ExportError;

/// Create a canonical lossless backup ZIP.
///
/// When `key` is `Some`, `library.json` is sealed with the library data key and
/// the manifest records the AEAD envelope; binaries remain plaintext in the
/// archive (at-rest binary sealing is deferred to a later phase). With `None`,
/// a fully plaintext backup is written — the default offline artifact.
pub fn export_backup(
    db: &Database,
    data_dir: &Path,
    output_path: &Path,
    key: Option<&SyncKey>,
) -> Result<(), ExportError> {
    let data = LibraryIo::new(db).export_library()?;
    let library_json = serde_json::to_string_pretty(&data)?;

    let (encrypted, envelope): (bool, Option<EncryptedEnvelope>) = match key {
        Some(k) => {
            let env = encrypt_snapshot(k, &library_json)
                .map_err(|e| ExportError::Crypto(e.to_string()))?;
            (true, Some(env))
        }
        None => (false, None),
    };

    let file = fs::File::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Payload: plaintext library.json, or nothing (sealed payload rides in the
    // manifest envelope).
    if !encrypted {
        zip.start_file("library.json", options)?;
        zip.write_all(library_json.as_bytes())?;
    }

    // Cover binaries, content-addressed and de-duplicated by hash.
    let covers_dir = data_dir.join("covers");
    let mut written_covers: HashSet<String> = HashSet::new();
    for book in &data.books {
        if let Some(hash) = &book.cover_hash
            && written_covers.insert(hash.clone())
        {
            let cover_path = covers_dir.join(format!("{hash}.jpg"));
            if cover_path.exists() {
                let bytes = fs::read(&cover_path)?;
                zip.start_file(format!("covers/{hash}.jpg"), options)?;
                zip.write_all(&bytes)?;
            }
        }
    }

    // Ebook binaries, content-addressed by checksum and de-duplicated.
    let mut written_files: HashSet<String> = HashSet::new();
    for f in &data.files {
        let entry = format!("files/{}.{}", f.checksum, f.format);
        if !written_files.insert(entry.clone()) {
            continue;
        }
        let src = Path::new(&f.path);
        if src.exists() {
            let bytes = fs::read(src)?;
            zip.start_file(entry, options)?;
            zip.write_all(&bytes)?;
        }
    }

    let mut counts = data.counts();
    counts.covers = written_covers.len();
    let manifest = BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        created_at: chrono::Utc::now().to_rfc3339(),
        counts,
        encrypted,
        envelope,
    };
    zip.start_file("manifest.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

    zip.finish()?;
    Ok(())
}

/// Read and validate the manifest of a backup ZIP without restoring anything.
///
/// Used for `--dry-run` and to surface encryption/version state before a write.
pub fn read_backup_manifest(zip_path: &Path) -> Result<BackupManifest, ExportError> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    parse_manifest(&mut archive)
}

/// Restore a canonical backup into the database (ADR-012 D3).
///
/// `mode` selects merge (default, additive, precedence-respecting) or replace
/// (verbatim into a cleared library). `key` decrypts a sealed `library.json`.
/// Binaries are extracted content-addressed after the database transaction
/// commits; extraction is idempotent (existing content is left in place).
pub fn import_backup(
    zip_path: &Path,
    db: &Database,
    data_dir: &Path,
    mode: RestoreMode,
    key: Option<&SyncKey>,
) -> Result<RestoreResult, ExportError> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let manifest = parse_manifest(&mut archive)?;

    let library_json = if manifest.encrypted {
        let envelope = manifest
            .envelope
            .as_ref()
            .ok_or_else(|| ExportError::Malformed("encrypted backup has no envelope".into()))?;
        let key = key.ok_or_else(|| {
            ExportError::Crypto(
                "backup is encrypted but no library data key is available to decrypt it".into(),
            )
        })?;
        decrypt_snapshot(key, envelope).map_err(|e| ExportError::Crypto(e.to_string()))?
    } else {
        read_zip_entry_string(&mut archive, "library.json")?
    };

    let data: LibraryData = serde_json::from_str(&library_json)
        .map_err(|e| ExportError::Malformed(format!("library.json: {e}")))?;

    let result = LibraryIo::new(db).restore_library(&data, mode)?;

    extract_binaries(&mut archive, data_dir, &data)?;

    Ok(result)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Parse and version-check `manifest.json`, distinguishing a superseded v1 flat
/// archive from a current v2 lossless one.
fn parse_manifest<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<BackupManifest, ExportError> {
    let raw = read_zip_entry_string(archive, "manifest.json")?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| ExportError::Malformed(format!("manifest.json: {e}")))?;

    // A v1 flat backup carries a string `version` and no `format_version`.
    if value.get("format_version").is_none() {
        if value.get("version").is_some() {
            return Err(ExportError::UnsupportedVersion(1, BACKUP_FORMAT_VERSION));
        }
        return Err(ExportError::Malformed(
            "manifest.json is missing format_version".into(),
        ));
    }

    let manifest: BackupManifest = serde_json::from_value(value)
        .map_err(|e| ExportError::Malformed(format!("manifest.json: {e}")))?;

    if manifest.format_version < BACKUP_FORMAT_VERSION {
        // Only v1 exists below the current version; treat any sub-current value
        // as the superseded flat format.
        return Err(ExportError::UnsupportedVersion(
            manifest.format_version,
            BACKUP_FORMAT_VERSION,
        ));
    }
    if manifest.format_version > BACKUP_FORMAT_VERSION {
        return Err(ExportError::FutureVersion(
            manifest.format_version,
            BACKUP_FORMAT_VERSION,
        ));
    }

    Ok(manifest)
}

fn read_zip_entry_string<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, ExportError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| ExportError::Malformed(format!("backup is missing {name}")))?;
    let mut s = String::new();
    entry.read_to_string(&mut s)?;
    Ok(s)
}

/// Extract cover and ebook binaries to their content-addressed locations,
/// skipping any content already present (dedup by hash/checksum, ADR-011).
fn extract_binaries<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    data_dir: &Path,
    data: &LibraryData,
) -> Result<(), ExportError> {
    // Covers → data_dir/covers/<hash>.jpg
    let covers_dir = data_dir.join("covers");
    let mut done: HashSet<String> = HashSet::new();
    for book in &data.books {
        if let Some(hash) = &book.cover_hash
            && done.insert(hash.clone())
        {
            let entry_name = format!("covers/{hash}.jpg");
            let dest = covers_dir.join(format!("{hash}.jpg"));
            if dest.exists() {
                continue;
            }
            if let Some(bytes) = read_zip_entry_bytes(archive, &entry_name)? {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&dest, bytes)?;
            }
        }
    }

    // Ebook files → their recorded path (reconstructs the on-disk library).
    let mut file_done: HashSet<String> = HashSet::new();
    for f in &data.files {
        let entry_name = format!("files/{}.{}", f.checksum, f.format);
        if !file_done.insert(entry_name.clone()) {
            continue;
        }
        let dest = Path::new(&f.path);
        if dest.exists() {
            continue;
        }
        if let Some(bytes) = read_zip_entry_bytes(archive, &entry_name)? {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(dest, bytes)?;
        }
    }

    Ok(())
}

fn read_zip_entry_bytes<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, ExportError> {
    match archive.by_name(name) {
        Ok(mut entry) => {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(ExportError::Zip(e)),
    }
}
