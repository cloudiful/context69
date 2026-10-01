//! Bounded snapshot indexing limits (issue #681 work unit 3B3).
//!
//! Acquisition already caps decoded bytes, tree entries, and response bodies.
//! The index layer adds the two counts a snapshot walk must never exceed — how
//! many eligible files one generation may include and how many distinct Blob
//! fetches it may spend — plus the chunk bounds each stored file is cut with.
//! A snapshot that would cross either count fails the whole generation instead
//! of activating partial content.

use std::collections::HashSet;

use anyhow::Result;

use crate::domain_errors::DomainError;

use super::code_text::CodeChunkBounds;
use super::model::{GitAcquisitionLimits, TreeFileEntry};

/// Largest number of eligible files one snapshot may include by default. It is
/// the storage layer's manifest ceiling, so the walk and the write agree on the
/// same bound instead of discovering the limit inside the statement.
pub(crate) const MAX_INDEX_FILES: usize = crate::db::MAX_GIT_MANIFEST_FILES;

/// Largest number of distinct Blob fetches one snapshot may perform by default.
/// Identical blob ids are fetched once, so this is a request budget rather than
/// a file count.
pub(crate) const MAX_INDEX_BLOB_FETCHES: usize = crate::db::MAX_GIT_MANIFEST_FILES;

/// Complete limit set for one bounded snapshot.
///
/// The acquisition limits stay separate from the index counts: the former
/// bound decoded bytes, tree entries, and response bodies per request, while
/// `max_files` and `max_blob_fetches` bound how much of a listing the indexer
/// will ever walk. Exceeding the index counts fails closed before activation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GitIndexLimits {
    /// Byte, tree-entry, and response ceilings applied to the acquirer of one
    /// generation.
    pub acquisition: GitAcquisitionLimits,
    /// Largest number of eligible files one snapshot may include.
    pub max_files: usize,
    /// Largest number of distinct Blob fetches one snapshot may perform.
    pub max_blob_fetches: usize,
    /// Bounds every stored file's chunks are cut with.
    pub chunk_bounds: CodeChunkBounds,
}

impl Default for GitIndexLimits {
    fn default() -> Self {
        Self {
            acquisition: GitAcquisitionLimits::default(),
            max_files: MAX_INDEX_FILES,
            max_blob_fetches: MAX_INDEX_BLOB_FETCHES,
            chunk_bounds: CodeChunkBounds::default(),
        }
    }
}

/// Applies the explicit eligible-file and Blob-fetch counts to a listing.
///
/// The listing already excluded unsafe, symlinked, and unservable entries and
/// reports them as coverage; this layer refuses a snapshot that would exceed
/// how many files or distinct BLOBs one generation may walk, so an oversized
/// repository fails closed instead of activating a truncated manifest.
pub(crate) fn plan_snapshot_files(
    files: Vec<TreeFileEntry>,
    limits: &GitIndexLimits,
) -> Result<Vec<TreeFileEntry>> {
    if files.len() > limits.max_files {
        return Err(DomainError::payload_too_large("git_index_file_limit").into());
    }
    let distinct_blobs = {
        let mut seen: HashSet<&str> = HashSet::with_capacity(files.len());
        for file in &files {
            seen.insert(file.blob_sha.as_str());
        }
        seen.len()
    };
    if distinct_blobs > limits.max_blob_fetches {
        return Err(DomainError::payload_too_large("git_index_blob_limit").into());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::git_repository::ref_path_safety::{SafeSha, SafeTreePath};

    fn sha(seed: u8) -> String {
        format!("{seed:02x}").repeat(20)
    }

    fn entry(path: &str, blob: &str) -> TreeFileEntry {
        TreeFileEntry {
            path: SafeTreePath::parse(path).unwrap(),
            blob_sha: SafeSha::parse(blob).unwrap(),
            size: Some(0),
        }
    }

    #[test]
    fn eligible_file_and_blob_fetch_limits_fail_closed() {
        let two = vec![entry("a.rs", &sha(1)), entry("b.rs", &sha(2))];
        let tight_files = GitIndexLimits {
            max_files: 1,
            ..GitIndexLimits::default()
        };
        let error = plan_snapshot_files(two.clone(), &tight_files)
            .unwrap_err()
            .to_string();
        assert!(error.contains("git_index_file_limit"), "{error}");

        let tight_blobs = GitIndexLimits {
            max_blob_fetches: 1,
            ..GitIndexLimits::default()
        };
        let error = plan_snapshot_files(two, &tight_blobs)
            .unwrap_err()
            .to_string();
        assert!(error.contains("git_index_blob_limit"), "{error}");

        // Two paths sharing one blob id cost one fetch, so they fit the budget.
        let shared = vec![entry("a.rs", &sha(1)), entry("b.rs", &sha(1))];
        assert_eq!(plan_snapshot_files(shared, &tight_blobs).unwrap().len(), 2);

        // An empty listing is a valid, bounded snapshot.
        assert!(
            plan_snapshot_files(Vec::new(), &tight_files)
                .unwrap()
                .is_empty()
        );

        let defaults = GitIndexLimits::default();
        assert_eq!(defaults.max_files, MAX_INDEX_FILES);
        assert_eq!(defaults.max_blob_fetches, MAX_INDEX_BLOB_FETCHES);
    }
}
