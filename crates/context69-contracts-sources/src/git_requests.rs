//! Request contracts for public Git repository registration, explicit
//! connection attachment, and provider connection creation (issue #681 work
//! units 3C2, 4A2, and 4B3).
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
//!
//! [`GitProviderConnectionRequest`] is the create-only connection body: bounded
//! non-secret metadata plus one tri-state read-credential patch. It carries no
//! secret-store key, App-private-key field, webhook field, installation
//! identity, or provider token metadata, and it reuses
//! [`validate_git_connection_key`] for its path key.
//!
//! The read-credential patch mirrors the settings `SecretPatch` wire shape
//! (`{"op":"keep"}`, `{"op":"set","value":...}`, `{"op":"clear"}`). It is a
//! local type because this leaf crate does not depend on the settings contract
//! crate, and the shared value-echoing `Set(String)` is only ever consumed by
//! the server-side writer, never projected or logged.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::git_repositories::{
    GitConnectionMode, GitIndexProfile, GitProviderKind, GitRefreshPolicy,
};

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

/// Maximum characters accepted for a connection display name.
pub const GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS: usize = 200;

/// Maximum characters accepted for a connection provider base URL.
pub const GIT_CONNECTION_BASE_URL_MAX_CHARS: usize = 2048;

/// Tri-state read-credential patch for connection creation.
///
/// `Keep` (the default) creates the connection without a read credential,
/// `Set` seals the supplied value, and `Clear` is refused on create because a
/// row that does not exist yet has nothing to clear.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum GitReadCredentialPatch {
    #[default]
    Keep,
    Set(String),
    Clear,
}

impl GitReadCredentialPatch {
    /// Whether this patch leaves the connection uncredentialed.
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    /// Whether this patch asks to clear a credential a create cannot have.
    pub fn is_clear(&self) -> bool {
        matches!(self, Self::Clear)
    }

    /// The supplied credential when this is a non-blank `Set`.
    ///
    /// A blank `Set` yields `None`; the request validator refuses it before
    /// this is reached, so a caller can never silently create an
    /// uncredentialed connection while asking to set one.
    pub fn set_value(&self) -> Option<&str> {
        match self {
            Self::Set(value) if !value.trim().is_empty() => Some(value),
            _ => None,
        }
    }
}

/// Why a connection-create request was refused before any I/O.
///
/// Every variant maps to a stable bounded reason; the reason never echoes a
/// submitted value, so a credential cannot leak through an error body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitConnectionRequestRejection {
    /// `display_name` is empty or whitespace-only.
    DisplayNameBlank,
    /// `display_name` is longer than [`GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS`].
    DisplayNameTooLong,
    /// `base_url` is empty or whitespace-only.
    BaseUrlBlank,
    /// `base_url` is longer than [`GIT_CONNECTION_BASE_URL_MAX_CHARS`].
    BaseUrlTooLong,
    /// `base_url` is not an `http`/`https` URL.
    BaseUrlUnsupported,
    /// `read_credential` is a `Set` with a blank value.
    CredentialSetBlank,
    /// `read_credential` is `Clear`, which a create cannot honour.
    CredentialClearUnsupported,
}

impl GitConnectionRequestRejection {
    /// Stable bounded reason for API error bodies.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DisplayNameBlank => "git_connection_display_name_blank",
            Self::DisplayNameTooLong => "git_connection_display_name_too_long",
            Self::BaseUrlBlank => "git_connection_base_url_blank",
            Self::BaseUrlTooLong => "git_connection_base_url_too_long",
            Self::BaseUrlUnsupported => "git_connection_base_url_unsupported",
            Self::CredentialSetBlank => "git_connection_secret_set_blank",
            Self::CredentialClearUnsupported => "git_connection_secret_clear_unsupported_on_create",
        }
    }
}

/// Request body creating one group-owned Git provider connection.
///
/// The body carries bounded, non-secret metadata plus one tri-state
/// `read_credential` patch. It has no secret-store key, App-private-key field,
/// webhook field, installation identity, or provider token metadata, so
/// `deny_unknown_fields` rejects every other connection field outright. A
/// `Set` credential is sealed before the response is produced; a `Clear` is
/// refused because a create has nothing to clear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitProviderConnectionRequest {
    /// Provider family this connection authenticates against.
    pub provider: GitProviderKind,
    /// Authentication mode: public, installation, or token.
    pub mode: GitConnectionMode,
    /// Human-readable display name.
    #[schema(min_length = 1, max_length = 200)]
    #[schemars(length(min = 1, max = GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS))]
    pub display_name: String,
    /// Provider API base URL; must be an `http`/`https` URL.
    #[schema(min_length = 1, max_length = 2048)]
    #[schemars(length(min = 1, max = GIT_CONNECTION_BASE_URL_MAX_CHARS))]
    pub base_url: String,
    /// Tri-state read-credential patch; defaults to `Keep`.
    #[serde(default)]
    pub read_credential: GitReadCredentialPatch,
}

impl GitProviderConnectionRequest {
    /// Validates the bounded, non-secret fields before any group lookup or I/O.
    ///
    /// A blank or unsupported value and a `Clear` patch are refused with a
    /// stable reason. Lengths are counted in characters, matching the schema
    /// bounds, and no field is echoed back.
    pub fn validate_for_create(&self) -> Result<(), GitConnectionRequestRejection> {
        if self.display_name.trim().is_empty() {
            return Err(GitConnectionRequestRejection::DisplayNameBlank);
        }
        if self.display_name.chars().count() > GIT_CONNECTION_DISPLAY_NAME_MAX_CHARS {
            return Err(GitConnectionRequestRejection::DisplayNameTooLong);
        }
        let base_url = self.base_url.trim();
        if base_url.is_empty() {
            return Err(GitConnectionRequestRejection::BaseUrlBlank);
        }
        if self.base_url.chars().count() > GIT_CONNECTION_BASE_URL_MAX_CHARS {
            return Err(GitConnectionRequestRejection::BaseUrlTooLong);
        }
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(GitConnectionRequestRejection::BaseUrlUnsupported);
        }
        match &self.read_credential {
            GitReadCredentialPatch::Clear => {
                Err(GitConnectionRequestRejection::CredentialClearUnsupported)
            }
            GitReadCredentialPatch::Set(value) if value.trim().is_empty() => {
                Err(GitConnectionRequestRejection::CredentialSetBlank)
            }
            _ => Ok(()),
        }
    }
}

/// Validates a provider-connection key without touching the database or a
/// provider.
///
/// The accepted form is `1..=GIT_CONNECTION_KEY_MAX_CHARS` ASCII
/// alphanumerics, `-`, `_`, and `.`, not starting with `.`. Path separators,
/// whitespace, control bytes, and non-ASCII characters are refused, so neither
/// a secret-store reference nor a path form can be submitted as a connection
/// key. The returned key is the exact submitted value; nothing is trimmed or
/// case-folded, because a lookup must match the stored key byte for byte.
pub fn validate_git_connection_key(key: &str) -> Result<String, GitConnectionKeyRejection> {
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
    /// Delegates to [`validate_git_connection_key`], the one accepted key
    /// charset shared by connection attachment and connection creation.
    pub fn validated_connection_key(&self) -> Result<String, GitConnectionKeyRejection> {
        validate_git_connection_key(&self.connection_key)
    }
}
