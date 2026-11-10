//! Request contracts for public Git repository registration, explicit
//! connection attachment, provider connection creation, and repository webhook
//! registration (issue #681 work units 3C2, 4A2, 4B3, and phase 5H).
//!
//! Registration names the provider explicitly and may reference an existing
//! group-owned connection: the caller supplies a provider, a canonical URL, a
//! default branch, the target ref, and optionally a pinned commit, a connection
//! key, plus the independent index and refresh policies. A Forgejo registration
//! requires a connection, because only the connection can establish the
//! instance origin the canonical URL must match. Owner and name are derived
//! server-side from the validated URL, so a request can never widen provider
//! access or rename the repository it targets, and the body never carries a
//! credential value or secret-store key.
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
//!
//! [`GitWebhookRegistrationRequest`] is the create-only webhook body: a provider
//! that must match the repository's own, a bounded provider-issued hook id, the
//! existing ownership enum, and one optional plain signing secret. The secret is
//! a value on its way into the encrypted store — never a store key or reference —
//! and its presence, not a submitted flag, decides whether the hook is active.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::git_repositories::{
    GitConnectionMode, GitIndexProfile, GitProviderKind, GitWebhookOwnership,
};

fn default_index_profile() -> GitIndexProfile {
    GitIndexProfile::Lexical
}

fn default_enabled() -> bool {
    true
}

fn default_retention_count() -> i64 {
    2
}

/// Largest number of user include or exclude glob patterns one source keeps.
pub const GIT_SOURCE_PATTERN_MAX_COUNT: usize = 128;

/// Longest accepted user include or exclude glob pattern, in characters.
pub const GIT_SOURCE_PATTERN_MAX_CHARS: usize = 512;

/// Smallest accepted independent periodic interval, in seconds.
pub const GIT_REFRESH_INTERVAL_MIN_SECONDS: i64 = 300;

/// Largest accepted independent periodic interval, in seconds (30 days).
pub const GIT_REFRESH_INTERVAL_MAX_SECONDS: i64 = 2_592_000;

/// Smallest accepted retention count: only the active snapshot is retained.
pub const GIT_RETENTION_MIN: i64 = 1;

/// Largest accepted retention count.
pub const GIT_RETENTION_MAX: i64 = 50;

/// Default retention count: the active generation plus one previous successful
/// snapshot, counted as the total retained snapshots.
pub const GIT_RETENTION_DEFAULT: i64 = 2;

/// Optional body of the group Git index request.
///
/// Normal sync keeps the same-target no-op. `force_rebuild` bypasses that
/// no-op and incremental reuse so the current target is reconstructed even when
/// its commit and pipeline identity are unchanged.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct GitIndexRequest {
    #[serde(default)]
    pub force_rebuild: bool,
}

/// Independent, user-editable settings of one repository source.
///
/// The fields are the complete settings surface, so registration and the
/// settings update share one shape: the bounded default allowlist stays in
/// effect unless an include adds a path, a user exclude always wins, the
/// periodic interval is optional and independent of the webhook switch, and
/// retention is a bounded count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitRepositorySettingsRequest {
    /// Additional repository-relative glob patterns admitted beyond the
    /// bounded default source/docs/manifest allowlist.
    #[serde(default)]
    #[schema(max_items = 128)]
    pub include_patterns: Vec<String>,
    /// Glob patterns excluded even when the default allowlist or an include
    /// matches. Safety exclusions cannot be re-enabled.
    #[serde(default)]
    #[schema(max_items = 128)]
    pub exclude_patterns: Vec<String>,
    /// Optional periodic refresh interval, in whole seconds (300..2592000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<i64>,
    /// Whether authenticated provider push events refresh this source.
    #[serde(default)]
    pub webhook_enabled: bool,
    /// Whether this source accepts index work.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Total successful snapshots to retain, including the active one (1..50).
    #[serde(default = "default_retention_count")]
    pub retention_count: i64,
}

impl Default for GitRepositorySettingsRequest {
    fn default() -> Self {
        Self {
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            interval_seconds: None,
            webhook_enabled: false,
            enabled: true,
            retention_count: GIT_RETENTION_DEFAULT,
        }
    }
}

impl GitRepositorySettingsRequest {
    /// Validates every bounded setting before any storage or provider work.
    ///
    /// Patterns are trimmed, counted in characters, and refused when blank or
    /// when they carry control characters; duplicate patterns are collapsed so
    /// one source cannot store the same rule twice. The interval is either
    /// absent or inside the supported range, and retention stays inside its own
    /// small bound. No submitted value is echoed back through the error.
    pub fn validated(self) -> Result<Self, GitRepositorySettingsRejection> {
        let include_patterns = validated_patterns(self.include_patterns)?;
        let exclude_patterns = validated_patterns(self.exclude_patterns)?;
        if let Some(interval) = self.interval_seconds
            && !(GIT_REFRESH_INTERVAL_MIN_SECONDS..=GIT_REFRESH_INTERVAL_MAX_SECONDS)
                .contains(&interval)
        {
            return Err(GitRepositorySettingsRejection::IntervalOutOfRange);
        }
        if !(GIT_RETENTION_MIN..=GIT_RETENTION_MAX).contains(&self.retention_count) {
            return Err(GitRepositorySettingsRejection::RetentionOutOfRange);
        }
        Ok(Self {
            include_patterns,
            exclude_patterns,
            interval_seconds: self.interval_seconds,
            webhook_enabled: self.webhook_enabled,
            enabled: self.enabled,
            retention_count: self.retention_count,
        })
    }
}

/// Explicit pinned-commit edit on a source settings update.
///
/// The tri-state form is required because an omitted field means "keep the
/// current pin" while JSON `null` would be indistinguishable from an absent
/// value after deserialization. `Clear` removes the pin so the next run
/// resolves the tracked ref again.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum GitPinnedCommitPatch {
    #[default]
    Keep,
    Set(String),
    Clear,
}

impl GitPinnedCommitPatch {
    /// Stable action name understood by the persistence statement.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Set(_) => "set",
            Self::Clear => "clear",
        }
    }

    /// The supplied pin when this is a non-blank `Set`.
    pub fn set_value(&self) -> Option<&str> {
        match self {
            Self::Set(value) if !value.trim().is_empty() => Some(value),
            _ => None,
        }
    }
}

/// Editable target, profile, and settings of one registered repository.
///
/// Registration fixes the provider identity and canonical URL. A later patch may
/// move the tracked ref, pin or clear an exact commit, and change the index
/// profile, so those are explicit fields here rather than being fixed forever at
/// registration time. The shared settings fields keep the one bounded validation
/// surface used by registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitRepositorySettingsUpdateRequest {
    /// New full target ref (`refs/heads/main`, `refs/tags/v1`, or `HEAD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<String>,
    /// Pin edit; omission keeps the current pin and `clear` removes it.
    #[serde(default)]
    pub pinned_commit: GitPinnedCommitPatch,
    /// New index profile; omission keeps the current profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_profile: Option<GitIndexProfile>,
    #[serde(default)]
    #[schema(max_items = 128)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    #[schema(max_items = 128)]
    pub exclude_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<i64>,
    #[serde(default)]
    pub webhook_enabled: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_retention_count")]
    pub retention_count: i64,
}

impl Default for GitRepositorySettingsUpdateRequest {
    fn default() -> Self {
        Self {
            target_ref: None,
            pinned_commit: GitPinnedCommitPatch::Keep,
            index_profile: None,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            interval_seconds: None,
            webhook_enabled: false,
            enabled: true,
            retention_count: GIT_RETENTION_DEFAULT,
        }
    }
}

impl GitRepositorySettingsUpdateRequest {
    /// Validates the shared settings surface and returns the normalized request.
    pub fn validated(self) -> Result<Self, GitRepositorySettingsRejection> {
        let settings = GitRepositorySettingsRequest {
            include_patterns: self.include_patterns,
            exclude_patterns: self.exclude_patterns,
            interval_seconds: self.interval_seconds,
            webhook_enabled: self.webhook_enabled,
            enabled: self.enabled,
            retention_count: self.retention_count,
        }
        .validated()?;
        Ok(Self {
            target_ref: self.target_ref,
            pinned_commit: self.pinned_commit,
            index_profile: self.index_profile,
            include_patterns: settings.include_patterns,
            exclude_patterns: settings.exclude_patterns,
            interval_seconds: settings.interval_seconds,
            webhook_enabled: settings.webhook_enabled,
            enabled: settings.enabled,
            retention_count: settings.retention_count,
        })
    }
}

fn validated_patterns(
    patterns: Vec<String>,
) -> Result<Vec<String>, GitRepositorySettingsRejection> {
    if patterns.len() > GIT_SOURCE_PATTERN_MAX_COUNT {
        return Err(GitRepositorySettingsRejection::TooManyPatterns);
    }
    let mut validated: Vec<String> = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Err(GitRepositorySettingsRejection::BlankPattern);
        }
        if pattern.chars().count() > GIT_SOURCE_PATTERN_MAX_CHARS {
            return Err(GitRepositorySettingsRejection::PatternTooLong);
        }
        if pattern.chars().any(char::is_control) {
            return Err(GitRepositorySettingsRejection::PatternControlCharacter);
        }
        let pattern = pattern.to_string();
        if !validated.contains(&pattern) {
            validated.push(pattern);
        }
    }
    Ok(validated)
}

/// Why a settings request was refused before any persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepositorySettingsRejection {
    TooManyPatterns,
    BlankPattern,
    PatternTooLong,
    PatternControlCharacter,
    IntervalOutOfRange,
    RetentionOutOfRange,
}

impl GitRepositorySettingsRejection {
    /// Stable bounded reason for API error bodies.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooManyPatterns => "git_source_patterns_too_many",
            Self::BlankPattern => "git_source_pattern_blank",
            Self::PatternTooLong => "git_source_pattern_too_long",
            Self::PatternControlCharacter => "git_source_pattern_control_character",
            Self::IntervalOutOfRange => "git_source_interval_out_of_range",
            Self::RetentionOutOfRange => "git_source_retention_out_of_range",
        }
    }
}

/// Registration request for one repository ref on an explicit provider.
///
/// A GitHub registration may be public or attach an existing GitHub connection,
/// and its `canonical_url` must be exactly `https://github.com/{owner}/{name}`.
/// A Forgejo registration must attach a connection whose origin the canonical
/// `https://{origin}/{owner}/{name}` URL matches. A trailing slash and a single
/// `.git` suffix are tolerated by the validator. `target_ref` is a full ref
/// (`refs/heads/main`, `refs/tags/v1`) or `HEAD`, and when it names a branch it
/// must match `default_branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryRegistrationRequest {
    /// Provider family this repository is acquired from.
    pub provider: GitProviderKind,
    /// Canonical HTTPS repository URL on the provider origin.
    pub canonical_url: String,
    /// Default branch name of the repository.
    pub default_branch: String,
    /// Full target ref to index, or `HEAD`.
    pub target_ref: String,
    /// Optional pinned commit SHA the target ref must resolve to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_commit: Option<String>,
    /// Optional existing group-owned connection used for acquisition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_key: Option<String>,
    /// Independent index profile; defaults to cheap lexical indexing.
    #[serde(default = "default_index_profile")]
    pub index_profile: GitIndexProfile,
    /// Independent selection and refresh settings for the source.
    #[serde(flatten)]
    pub settings: GitRepositorySettingsRequest,
}

impl GitRepositoryRegistrationRequest {
    /// Validates the optional connection key without touching storage. A blank
    /// key is absent; a supplied key must use the shared connection-key charset.
    pub fn validated_connection_key(&self) -> Result<Option<String>, GitConnectionKeyRejection> {
        match self.connection_key.as_deref() {
            None => Ok(None),
            Some(key) if key.trim().is_empty() => Err(GitConnectionKeyRejection::Blank),
            Some(key) => validate_git_connection_key(key).map(Some),
        }
    }
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

/// Why a registration request was refused before any persistence or provider
/// work. Every variant names a stable bounded reason and never echoes a
/// submitted URL, key, or credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepositoryRegistrationRejection {
    /// The provider has no acquisition implementation in this phase.
    ProviderUnsupported,
    /// The provider requires an origin-bearing connection.
    ConnectionRequired,
    /// The referenced connection does not exist in the calling group.
    ConnectionNotFound,
    /// The connection belongs to another provider family.
    ConnectionProviderMismatch,
    /// The connection is disabled.
    ConnectionDisabled,
    /// The connection origin does not match the canonical repository URL.
    ConnectionOriginMismatch,
    /// The connection mode cannot acquire this provider's repositories.
    ConnectionModeUnsupported,
    /// A token connection has no stored read credential.
    ConnectionCredentialMissing,
}

impl GitRepositoryRegistrationRejection {
    /// Stable bounded reason for API error bodies.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderUnsupported => "git_provider_unsupported",
            Self::ConnectionRequired => "git_connection_required",
            Self::ConnectionNotFound => "git_connection_not_found",
            Self::ConnectionProviderMismatch => "git_connection_provider_mismatch",
            Self::ConnectionDisabled => "git_connection_disabled",
            Self::ConnectionOriginMismatch => "git_connection_origin_mismatch",
            Self::ConnectionModeUnsupported => "git_connection_mode_unsupported",
            Self::ConnectionCredentialMissing => "git_connection_credential_missing",
        }
    }
}

/// Maximum UTF-8 bytes accepted for a provider-issued webhook hook id, the same
/// bound the signed ingress applies to its own path segment: a multibyte id can
/// exceed it while still fitting 255 characters, and a row stored under a
/// character bound would be one the ingress can never resolve. The public name
/// keeps its `CHARS` suffix so no call site had to change over a unit change.
pub const GIT_WEBHOOK_HOOK_ID_MAX_CHARS: usize = 255;

/// Request body registering one webhook for a group-owned repository source.
///
/// The provider must match the repository's own, the hook id is provider-issued,
/// and the signing secret is accepted only to be sealed immediately: there is no
/// echo field, no secret-store key or reference, and no `active` flag, so
/// `deny_unknown_fields` keeps every internal field out of the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitWebhookRegistrationRequest {
    /// Provider family the hook belongs to; must match the repository's.
    pub provider: GitProviderKind,
    /// Provider-issued hook id, bounded to the ingress limit in UTF-8 bytes.
    // The utoipa derive needs a literal, so it mirrors the constant the runtime
    // validator uses; the OpenAPI bound is asserted in `src/api/docs.rs`.
    #[schema(min_length = 1, max_length = 255)]
    #[schemars(length(min = 1, max = GIT_WEBHOOK_HOOK_ID_MAX_CHARS))]
    pub external_hook_id: String,
    /// Whether this integration owns the hook it registered.
    pub ownership: GitWebhookOwnership,
    /// Optional plain signing secret, sealed immediately and never returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
}

/// Why a registration request was refused before any group-scoped work. Every
/// variant names a stable bounded reason and never echoes a submitted value, so
/// a signing secret cannot leak through an error body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitWebhookRegistrationRejection {
    /// `external_hook_id` is empty or whitespace-only.
    HookIdBlank,
    /// `external_hook_id` is longer than [`GIT_WEBHOOK_HOOK_ID_MAX_CHARS`] UTF-8
    /// bytes, which is the bound the signed ingress applies to the same id.
    HookIdTooLong,
    /// `signing_secret` was supplied blank or whitespace-only.
    SigningSecretBlank,
}

impl GitWebhookRegistrationRejection {
    /// Stable bounded reason for API error bodies.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HookIdBlank => "git_webhook_hook_id_blank",
            Self::HookIdTooLong => "git_webhook_hook_id_too_long",
            Self::SigningSecretBlank => "git_webhook_signing_secret_blank",
        }
    }
}

impl GitWebhookRegistrationRequest {
    /// Validates the bounded fields before any group lookup, secret write, or
    /// provider-facing work. The hook id is measured in UTF-8 bytes, exactly as
    /// the ingress measures its path segment; no field is echoed back.
    pub fn validate_for_create(&self) -> Result<(), GitWebhookRegistrationRejection> {
        if self.external_hook_id.trim().is_empty() {
            return Err(GitWebhookRegistrationRejection::HookIdBlank);
        }
        if self.external_hook_id.len() > GIT_WEBHOOK_HOOK_ID_MAX_CHARS {
            return Err(GitWebhookRegistrationRejection::HookIdTooLong);
        }
        match &self.signing_secret {
            Some(secret) if secret.trim().is_empty() => {
                Err(GitWebhookRegistrationRejection::SigningSecretBlank)
            }
            _ => Ok(()),
        }
    }

    /// The exact bytes to seal, or `None` for an omitted secret.
    ///
    /// The value is returned untrimmed: a provider verifies a signature over the
    /// exact bytes it was configured with, so a trimmed value would store a secret
    /// no delivery verifies against. A blank supplied secret is refused before
    /// this is reached.
    pub fn signing_secret_value(&self) -> Option<&str> {
        self.signing_secret
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    }

    /// Whether a create with this request stores an active registration.
    ///
    /// A sealed signing secret is what makes a hook verifiable, so a registration
    /// without one is stored inactive and the ingress records `Ignored` for it
    /// rather than failing open.
    pub fn is_active(&self) -> bool {
        self.signing_secret_value().is_some()
    }
}
