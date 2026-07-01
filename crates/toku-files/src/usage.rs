//! Disk-usage aggregation over associated ebook files.
//!
//! These helpers are pure — they aggregate over already-loaded [`EbookFile`]
//! records and never touch disk. Usage reflects the *catalog* view (each file's
//! stored `size_bytes`), which is deterministic and cheap; integrity of the
//! bytes on disk is a separate concern handled by [`crate::verify`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{EbookFile, FileFormat};

/// Aggregate totals across a set of files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub file_count: u64,
    pub total_bytes: i64,
}

impl UsageTotals {
    fn add(&mut self, size_bytes: i64) {
        self.file_count += 1;
        self.total_bytes += size_bytes;
    }
}

/// Compute overall totals (file count and summed size) for a set of files.
pub fn usage_totals(files: &[EbookFile]) -> UsageTotals {
    let mut totals = UsageTotals::default();
    for f in files {
        totals.add(f.size_bytes);
    }
    totals
}

/// Break usage down by file format. Formats with no files are omitted; the
/// map is ordered by [`FileFormat`]'s declaration order for stable output.
pub fn usage_by_format(files: &[EbookFile]) -> BTreeMap<String, UsageTotals> {
    usage_by_key(files, |f| vec![f.format.to_string()])
}

/// Break usage down by an arbitrary caller-supplied key.
///
/// `key_fn` returns *zero or more* keys for a file, so a single file can be
/// attributed to multiple buckets (e.g. every author or shelf it belongs to).
/// When it returns no keys, the file is skipped by this grouping — callers that
/// want an "(unassigned)" bucket should return that label themselves.
pub fn usage_by_key<F>(files: &[EbookFile], key_fn: F) -> BTreeMap<String, UsageTotals>
where
    F: Fn(&EbookFile) -> Vec<String>,
{
    let mut map: BTreeMap<String, UsageTotals> = BTreeMap::new();
    for f in files {
        for key in key_fn(f) {
            map.entry(key).or_default().add(f.size_bytes);
        }
    }
    map
}

/// Convenience: keep [`FileFormat`] as a typed key where callers want it.
pub fn usage_by_format_typed(files: &[EbookFile]) -> BTreeMap<FileFormat, UsageTotals> {
    let mut map: BTreeMap<FileFormat, UsageTotals> = BTreeMap::new();
    for f in files {
        map.entry(f.format).or_default().add(f.size_bytes);
    }
    map
}

impl std::cmp::PartialOrd for FileFormat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::Ord for FileFormat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn file(format: FileFormat, size: i64) -> EbookFile {
        EbookFile::new(
            Uuid::now_v7(),
            format!("/tmp/x.{}", format.as_str()),
            format,
            size,
            "deadbeef".to_string(),
        )
    }

    #[test]
    fn totals_sum_count_and_bytes() {
        let files = vec![
            file(FileFormat::Epub, 100),
            file(FileFormat::Pdf, 250),
            file(FileFormat::Epub, 50),
        ];
        let t = usage_totals(&files);
        assert_eq!(t.file_count, 3);
        assert_eq!(t.total_bytes, 400);
    }

    #[test]
    fn totals_of_empty_is_zero() {
        let t = usage_totals(&[]);
        assert_eq!(t, UsageTotals::default());
    }

    #[test]
    fn per_format_groups_and_sums() {
        let files = vec![
            file(FileFormat::Epub, 100),
            file(FileFormat::Pdf, 250),
            file(FileFormat::Epub, 50),
        ];
        let by = usage_by_format(&files);
        assert_eq!(by.get("epub").unwrap().file_count, 2);
        assert_eq!(by.get("epub").unwrap().total_bytes, 150);
        assert_eq!(by.get("pdf").unwrap().file_count, 1);
        assert_eq!(by.get("pdf").unwrap().total_bytes, 250);
        assert!(by.get("mobi").is_none());
    }

    #[test]
    fn by_key_attributes_file_to_multiple_buckets() {
        let files = vec![file(FileFormat::Epub, 100)];
        let by = usage_by_key(&files, |_| vec!["A".to_string(), "B".to_string()]);
        assert_eq!(by.get("A").unwrap().total_bytes, 100);
        assert_eq!(by.get("B").unwrap().total_bytes, 100);
    }

    #[test]
    fn by_key_skips_files_with_no_keys() {
        let files = vec![file(FileFormat::Epub, 100)];
        let by = usage_by_key(&files, |_| Vec::new());
        assert!(by.is_empty());
    }
}
