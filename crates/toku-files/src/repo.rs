//! Persistence for ebook file associations (`files` table).

use chrono::Utc;
use rusqlite::params;
use toku_db::Database;
use uuid::Uuid;

use crate::{EbookFile, FileError, FileFormat};

/// Repository for ebook file associations. Borrows a [`Database`] connection.
pub struct FileRepository<'a> {
    db: &'a Database,
}

impl<'a> FileRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Associate a file with a book. Errors if a file with the same checksum is
    /// already linked to the same book.
    pub fn add_file(&self, file: &EbookFile) -> Result<(), FileError> {
        if self
            .find_by_checksum(&file.book_id, &file.checksum)?
            .is_some()
        {
            return Err(FileError::Duplicate(file.checksum.clone()));
        }
        self.db.conn.execute(
            "INSERT INTO files (id, book_id, path, format, size_bytes, checksum,
             source, source_ref, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                file.id.to_string(),
                file.book_id.to_string(),
                file.path,
                file.format.as_str(),
                file.size_bytes,
                file.checksum,
                file.source,
                file.source_ref,
                file.created_at.to_rfc3339(),
                file.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// List every file association in the library, ordered by book then format.
    ///
    /// Used by integrity verification (`verify --all`) and disk-usage reporting,
    /// which operate across the whole catalog rather than a single book.
    pub fn list_all_files(&self) -> Result<Vec<EbookFile>, FileError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, book_id, path, format, size_bytes, checksum, source, source_ref, created_at, updated_at
             FROM files ORDER BY book_id, format",
        )?;
        let files = stmt
            .query_map([], |row| Ok(row_to_file(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
        Ok(files)
    }

    /// List all files associated with a book, ordered by format.
    pub fn list_files(&self, book_id: &Uuid) -> Result<Vec<EbookFile>, FileError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, book_id, path, format, size_bytes, checksum, source, source_ref, created_at, updated_at
             FROM files WHERE book_id = ?1 ORDER BY format",
        )?;
        let files = stmt
            .query_map(params![book_id.to_string()], |row| Ok(row_to_file(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
        Ok(files)
    }

    /// Find a file linked to a book by exact checksum.
    pub fn find_by_checksum(
        &self,
        book_id: &Uuid,
        checksum: &str,
    ) -> Result<Option<EbookFile>, FileError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, book_id, path, format, size_bytes, checksum, source, source_ref, created_at, updated_at
             FROM files WHERE book_id = ?1 AND checksum = ?2",
        )?;
        let mut rows = stmt
            .query_map(params![book_id.to_string(), checksum], |row| {
                Ok(row_to_file(row))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok());
        Ok(rows.next())
    }

    /// Remove a file association by format. Returns the removed file, if any.
    pub fn remove_by_format(
        &self,
        book_id: &Uuid,
        format: FileFormat,
    ) -> Result<Option<EbookFile>, FileError> {
        let existing: Vec<EbookFile> = self
            .list_files(book_id)?
            .into_iter()
            .filter(|f| f.format == format)
            .collect();
        match existing.first() {
            Some(file) => {
                self.delete_record(&file.id)?;
                Ok(Some(file.clone()))
            }
            None => Ok(None),
        }
    }

    /// Remove a file association by exact path. Returns the removed file, if any.
    pub fn remove_by_path(
        &self,
        book_id: &Uuid,
        path: &str,
    ) -> Result<Option<EbookFile>, FileError> {
        let existing: Vec<EbookFile> = self
            .list_files(book_id)?
            .into_iter()
            .filter(|f| f.path == path)
            .collect();
        match existing.first() {
            Some(file) => {
                self.delete_record(&file.id)?;
                Ok(Some(file.clone()))
            }
            None => Ok(None),
        }
    }

    fn delete_record(&self, id: &Uuid) -> Result<(), FileError> {
        self.db
            .conn
            .execute("DELETE FROM files WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    /// Update the stored on-disk path of a file record and bump `updated_at`.
    pub fn update_path(&self, id: &Uuid, new_path: &str) -> Result<(), FileError> {
        self.db.conn.execute(
            "UPDATE files SET path = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), new_path, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
}

fn row_to_file(row: &rusqlite::Row<'_>) -> Result<EbookFile, String> {
    let id_str: String = row.get(0).map_err(|e| e.to_string())?;
    let book_id_str: String = row.get(1).map_err(|e| e.to_string())?;
    let format_str: String = row.get(3).map_err(|e| e.to_string())?;
    let created_str: String = row.get(8).map_err(|e| e.to_string())?;
    let updated_str: String = row.get(9).map_err(|e| e.to_string())?;
    Ok(EbookFile {
        id: Uuid::parse_str(&id_str).map_err(|e| e.to_string())?,
        book_id: Uuid::parse_str(&book_id_str).map_err(|e| e.to_string())?,
        path: row.get(2).map_err(|e| e.to_string())?,
        format: format_str
            .parse()
            .map_err(|_| format!("bad format: {format_str}"))?,
        size_bytes: row.get(4).map_err(|e| e.to_string())?,
        checksum: row.get(5).map_err(|e| e.to_string())?,
        source: row.get(6).map_err(|e| e.to_string())?,
        source_ref: row.get(7).map_err(|e| e.to_string())?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
            .map_err(|e| e.to_string())?
            .with_timezone(&Utc),
    })
}
