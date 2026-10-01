//! Request contracts for public Git repository registration and explicit
//! connection attachment (issue #681 work units 3C2 and 4A2).
//!
//! Registration is public GitHub only and carries no credential or connection
//! key: the caller supplies a canonical URL, a default branch, the target ref,
//! and optionally a pinned commit plus the independent index and refresh
//! policies. Owner, name, and provider are derived server-side from the
//! validated URL, so a request can never widen provider access or rename the
//! repository it targets.
//!
//! [`GitRepositoryConnectionRequest`] only names a connection that already
//! exists in the same group. The accepted key charset excludes path
//! separators, whitespace, and control bytes, so a secret-store reference can
//! never be submitted here, and `deny_unknown_fields` keeps every other
//! connection field (credential, token, webhook secret) out of the body.

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

/// Maximum characters accepted for an existing provider-connection key.
///
/// The bound keeps one request body small and matches the shortest plausible
/// descriptive key with room to spare; a longer value is rejected before any
/// lookup instead of being trimmed into a different key.
pub const GIT_CONNECTION_KEY_MAX_CHARS: usize = 128;

/// Request body attaching one already-persisted provider connection to a
/// repository source.
///
/// The body names a connection that must already exist in the same group. It
/// carries no credential, token, secret-store key, provider mode, or base URL:
/// `deny_unknown_fields` rejects any other connection field outright, so
/// attaching a connection can never create, configure, or enable one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitRepositoryConnectionRequest {
    /// Key of an existing, enabled provider connection owned by the same group.
    // The utoipa derive needs a literal, so it mirrors the constant the runtime
    // validator uses; the OpenAPI schema bound is asserted in `src/api/docs.rs`.
    #[schema(min_length = 1, max_length = 128)]
    #[schemars(length(min = 1, max = GIT_CONNECTION_KEY_MAX_CHARS))]
    pub connection_key: String,
}

/// Why a supplied connection key was refused before any group-scoped lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitConnectionKeyRejection {
    /// Empty or whitespace-only.
    Blank,
    /// Longer than [`GIT_CONNECTION_KEY_MAX_CHARS`].
    TooLong,
    /// Outside the accepted key charset, or a `.`/`..` style path form.
    Unsafe,
}

impl GitConnectionKeyRejection {
    /// Stable bounded reason for API error bodies; it never echoes the key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blank => "git_connection_key_blank",
            Self::TooLong => "git_connection_key_too_long",
            Self::Unsafe => "git_connection_key_invalid",
        }
    }
}

impl GitRepositoryConnectionRequest {
    /// Validates the supplied key without touching the database or a provider.
    ///
    /// The accepted form is `1..=GIT_CONNECTION_KEY_MAX_CHARS` ASCII
    /// alphanumerics, `-`, `_`, and `.`, not starting with `.`. Path
    /// separators, whitespace, control bytes, and non-ASCII characters are
    /// refused, so neither a secret-store reference nor a path form can be
    /// submitted as a connection key. The returned key is the exact submitted
    /// value; nothing is trimmed or case-folded, because a lookup must match
    /// the stored key byte for byte.
    pub fn validated_connection_key(&self) -> Result<String, GitConnectionKeyRejection> {
        let key = self.connection_key.as_str();
        if key.trim().is_empty() {
            return Err(GitConnectionKeyRejection::Blank);
        }
        if key.chars().count() > GIT_CONNECTION_KEY_MAX_CHARS {
            return Err(GitConnectionKeyRejection::TooLong);
        }
        let safe = !key.starts_with('.')
            && key.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            });
        if safe {
            Ok(key.to_owned())
        } else {
            Err(GitConnectionKeyRejection::Unsafe)
        }
    }
}
