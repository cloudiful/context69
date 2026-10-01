//! Public GitHub API response schemas for the bounded acquisition client.
//!
//! Each struct is a wire-format subset: unknown provider fields are ignored,
//! and every field the client relies on is validated further in
//! `github_client` (SHAs via `SafeSha`, paths via `SafeTreePath`).

use serde::Deserialize;

/// Wire-format subset of the GitHub `git/ref` response.
#[derive(Debug, Deserialize)]
pub(crate) struct RefResponse {
    pub object: RefObject,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RefObject {
    #[serde(rename = "type")]
    pub kind: String,
    pub sha: String,
}

/// Wire-format subset of the GitHub repository metadata response, reduced
/// to the default branch name that `HEAD` resolves through.
#[derive(Debug, Deserialize)]
pub(crate) struct RepoMetadataResponse {
    pub default_branch: String,
}

/// Wire-format subset of the GitHub `git/tags/{sha}` annotated-tag response:
/// the tag object with its target identifier and kind.
#[derive(Debug, Deserialize)]
pub(crate) struct TagResponse {
    pub sha: String,
    #[serde(rename = "object")]
    pub target: TagTarget,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TagTarget {
    pub sha: String,
    /// `commit`, `tag`, `tree`, or `blob`.
    #[serde(rename = "type")]
    pub kind: String,
}

/// Wire-format subset of the GitHub `git/trees` response. The provider
/// accepts a commit (or other) id and echoes it, so `sha` carries the
/// requested commit identity back, not a distinct tree id.
#[derive(Debug, Deserialize)]
pub(crate) struct TreeResponse {
    /// Echoed requested commit identity; validated by the client.
    pub sha: String,
    pub truncated: bool,
    pub tree: Vec<TreeEntryResponse>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TreeEntryResponse {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// File mode; `120000` marks a symlink whose blob is a link target, not
    /// file content, so it must never be indexed as a file.
    #[serde(default)]
    pub mode: Option<String>,
}

/// Wire-format subset of the GitHub `git/blobs` response.
#[derive(Debug, Deserialize)]
pub(crate) struct BlobResponse {
    pub sha: String,
    #[serde(default)]
    pub size: Option<u64>,
    pub content: String,
    pub encoding: String,
}
