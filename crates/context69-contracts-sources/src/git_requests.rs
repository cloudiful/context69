//! Request contracts for public Git repository registration (issue #681 work
//! unit 3C2).
//!
//! Registration is public GitHub only and carries no credential or connection
//! key: the caller supplies a canonical URL, a default branch, the target ref,
//! and optionally a pinned commit plus the independent index and refresh
//! policies. Owner, name, and provider are derived server-side from the
//! validated URL, so a request can never widen provider access or rename the
//! repository it targets.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::git_repositories::{GitIndexProfile, GitRefreshPolicy};

fn default_index_profile() -> GitIndexProfile {
    GitIndexProfile::Lexical
}

fn default_refresh_policy() -> GitRefreshPolicy {
    GitRefreshPolicy::Manual
}

/// Registration request for one public GitHub repository ref.
///
/// `canonical_url` must be exactly `https://github.com/{owner}/{name}` (a
/// trailing slash and a single `.git` suffix are tolerated by the validator).
/// `target_ref` is a full ref (`refs/heads/main`, `refs/tags/v1`) or `HEAD`,
/// and when it names a branch it must match `default_branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryRegistrationRequest {
    /// Canonical public GitHub HTTPS repository URL.
    pub canonical_url: String,
    /// Default branch name of the repository.
    pub default_branch: String,
    /// Full target ref to index, or `HEAD`.
    pub target_ref: String,
    /// Optional pinned commit SHA the target ref must resolve to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_commit: Option<String>,
    /// Independent index profile; defaults to cheap lexical indexing.
    #[serde(default = "default_index_profile")]
    pub index_profile: GitIndexProfile,
    /// Refresh policy; defaults to manual one-off snapshots.
    #[serde(default = "default_refresh_policy")]
    pub refresh_policy: GitRefreshPolicy,
}
