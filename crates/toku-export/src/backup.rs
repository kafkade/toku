use std::fs;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;

use toku_db::Database;

use crate::{ExportError, build_library_export};

/// Create a canonical backup ZIP containing manifest.json, library.json, and cover images.
pub fn export_backup(
    db: &Database,
    data_dir: &Path,
    output_path: &Path,
) -> Result<(), ExportError> {
    let export = build_library_export(db)?;

    let file = fs::File::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Write manifest.json
    let manifest = serde_json::json!({
        "version": export.version,
        "exported_at": export.exported_at,
        "book_count": export.book_count,
    });
    zip.start_file("manifest.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

    // Write library.json
    zip.start_file("library.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&export)?.as_bytes())?;

    // Copy cover images
    let covers_dir = data_dir.join("covers");
    if covers_dir.is_dir() {
        for book in &export.books {
            if let Some(hash) = &book.cover_hash {
                let cover_path = covers_dir.join(format!("{hash}.jpg"));
                if cover_path.exists() {
                    let cover_data = fs::read(&cover_path)?;
                    zip.start_file(format!("covers/{hash}.jpg"), options)?;
                    zip.write_all(&cover_data)?;
                }
            }
        }
    }

    zip.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use toku_core::{Book, BookFormat, ReadingStatus};
    use toku_db::BookRepository;

    #[test]
    fn backup_contains_manifest_and_library() {
        let db = Database::open_in_memory().unwrap();
        let repo = BookRepository::new(&db);

        let mut book = Book::new("Dune");
        book.status = ReadingStatus::Read;
        book.format = BookFormat::Physical;
        repo.create_book(&book).unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let output = tmp.path().join("backup.zip");

        export_backup(&db, &data_dir, &output).unwrap();

        let file = fs::File::open(&output).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();

        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();

        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.contains(&"library.json".to_string()));

        // Verify manifest parses
        {
            let mut manifest_file = archive.by_name("manifest.json").unwrap();
            let mut manifest_str = String::new();
            manifest_file.read_to_string(&mut manifest_str).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();
            assert_eq!(manifest["version"], "1");
            assert_eq!(manifest["book_count"], 1);
        }

        // Verify library.json round-trips
        {
            let mut lib_file = archive.by_name("library.json").unwrap();
            let mut lib_str = String::new();
            lib_file.read_to_string(&mut lib_str).unwrap();
            let parsed: crate::LibraryExport = serde_json::from_str(&lib_str).unwrap();
            assert_eq!(parsed.books.len(), 1);
            assert_eq!(parsed.books[0].title, "Dune");
        }
    }

    #[test]
    fn backup_includes_cover_files() {
        let db = Database::open_in_memory().unwrap();
        let repo = BookRepository::new(&db);

        let mut book = Book::new("Dune");
        book.status = ReadingStatus::Read;
        book.format = BookFormat::Physical;
        book.cover_hash = Some("abc123def456".to_string());
        repo.create_book(&book).unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let covers_dir = data_dir.join("covers");
        fs::create_dir_all(&covers_dir).unwrap();

        // Write a fake cover image
        let cover_content = b"fake-jpeg-data";
        fs::write(covers_dir.join("abc123def456.jpg"), cover_content).unwrap();

        let output = tmp.path().join("backup.zip");
        export_backup(&db, &data_dir, &output).unwrap();

        let file = fs::File::open(&output).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();

        let mut cover_file = archive.by_name("covers/abc123def456.jpg").unwrap();
        let mut cover_data = Vec::new();
        cover_file.read_to_end(&mut cover_data).unwrap();
        assert_eq!(cover_data, cover_content);
    }
}
