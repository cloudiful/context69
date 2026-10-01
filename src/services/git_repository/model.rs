//! Acquisition data model for the bounded public Git client (issue #681
//! phase 3A): hard ceilings, tree listing results, and decoded blob output.

use bytes::Bytes;

use super::ref_path_safety::{SafeSha, SafeTreePath};

/// Hard ceilings for one acquisition interaction. All limits are enforced
/// independently; the smallest applicable one wins.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GitAcquisitionLimits {
    /// Maximum decoded bytes for one blob.
    pub max_blob_bytes: usize,
    /// Maximum decoded bytes across all blobs of one acquisition.
    pub max_total_bytes: usize,
    /// Maximum body bytes for one API response.
    pub max_response_bytes: usize,
    /// Maximum number of tree entries accepted from one tree listing.
    pub max_tree_entries: usize,
}

impl Default for GitAcquisitionLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 8 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_response_bytes: 32 * 1024 * 1024,
            max_tree_entries: 100_000,
        }
    }
}

impl GitAcquisitionLimits {
    /// Per-blob decoded-byte budget.
    pub(crate) fn blob_budget(&self) -> super::bounds::ByteBudget {
        super::bounds::ByteBudget {
            max_bytes: self.max_blob_bytes,
        }
    }
}

/// One file entry of a recursive tree listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeFileEntry {
    /// Repository-relative file path.
    pub path: SafeTreePath,
    /// Blob SHA (lowercase hex) for later blob fetches.
    pub blob_sha: SafeSha,
    /// Declared blob size in bytes, when the provider reported one.
    pub size: Option<u64>,
}

/// Result of one recursive listing pinned to a commit.
#[derive(Debug, Clone)]
pub(crate) struct TreeListing {
    /// The commit identity this listing is pinned to: the requested commit
    /// SHA, echoed back by the provider. Downstream provenance (manifests,
    /// pinned snapshots) must carry this commit id, not a tree id; the
    /// provider response is validated to equal the requested commit.
    pub commit_sha: SafeSha,
    /// File (blob) entries only; directories, submodules, symlinks, and
    /// entries rejected by path/size policy are skipped, not returned.
    pub files: Vec<TreeFileEntry>,
    /// How many file entries the provider reported but this listing
    /// excluded (unsafe path, symlink, or unservable declared size), so
    /// partial coverage is visible instead of silent. Non-file entries
    /// (trees, submodules) are not counted: they were never files.
    pub excluded_files: usize,
}

/// Decoded blob content with its resolved metadata.
#[derive(Debug)]
pub(crate) struct BlobContent {
    pub blob_sha: SafeSha,
    pub declared_size: u64,
    pub bytes: Bytes,
}
