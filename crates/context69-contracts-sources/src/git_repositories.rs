use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use context69_contracts_core::Visibility;

/// Provider family behind a Git source or provider connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GitProviderKind {
    GitHub,
    Forgejo,
    GitLab,
    Generic,
}

impl GitProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Forgejo => "forgejo",
            Self::GitLab => "gitlab",
            Self::Generic => "generic",
        }
    }
}

/// How a Git provider connection authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitConnectionMode {
    /// Public, unauthenticated reads only.
    Public,
    /// A provider app installation owns repository access.
    Installation,
    /// An explicit least-privilege provider token fallback.
    Token,
}

impl GitConnectionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Installation => "installation",
            Self::Token => "token",
        }
    }
}

/// Version policy: the ref to read plus an optional pinned commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitVersionPolicy {
    pub ref_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}

/// Refresh policy for a tracked Git source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitRefreshPolicy {
    Manual,
    Webhook,
    Reconcile,
}

impl GitRefreshPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Webhook => "webhook",
            Self::Reconcile => "reconcile",
        }
    }
}

/// Independent index profile for a Git source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitIndexProfile {
    Lexical,
    Hybrid,
    FullSemantic,
}

impl GitIndexProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Hybrid => "hybrid",
            Self::FullSemantic => "full_semantic",
        }
    }
}

/// Index lifecycle state of a Git source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitIndexStatus {
    Pending,
    Indexing,
    Ready,
    Stale,
    Failed,
    Disabled,
}

impl GitIndexStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Indexing => "indexing",
            Self::Ready => "ready",
            Self::Stale => "stale",
            Self::Failed => "failed",
            Self::Disabled => "disabled",
        }
    }
}

/// Lifecycle state of one durable index generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitGenerationStatus {
    /// Claims are being acquired; the generation is not yet active.
    Building,
    /// Completed and activated as the repository's active generation.
    Ready,
    /// Abandoned after a failure; never activated.
    Failed,
    /// Was active and has been replaced by a newer generation.
    Superseded,
}

impl GitGenerationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
        }
    }
}

/// Incremental-sync checkpoint between the target and indexed commits.
///
/// `indexed_commit_sha` is the last fully indexed commit, so the next sync
/// diffs `indexed_commit_sha..target_commit_sha` instead of re-reading the ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitCommitCheckpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_commit_sha: Option<String>,
    /// When the indexed commit last advanced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<DateTime<Utc>>,
    /// When either side of the checkpoint last changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_updated_at: Option<DateTime<Utc>>,
}

/// A registered Git repository source with its owning group, identity,
/// policies, and checkpoint.
///
/// `group_key`, `group_path`, and `visibility` are read from the owning
/// `groups` row on every query, so a visibility change is reflected instead of
/// being served from a stored snapshot. Ownership is always a real group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositorySource {
    /// Key of the owning group; sources have no owner-less form.
    pub group_key: String,
    /// Current full path of the owning group.
    pub group_path: String,
    /// Current visibility of the owning group.
    pub visibility: Visibility,
    pub repository_key: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_key: Option<String>,
    pub provider: GitProviderKind,
    pub canonical_url: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub version: GitVersionPolicy,
    pub refresh: GitRefreshPolicy,
    pub index_profile: GitIndexProfile,
    pub index_status: GitIndexStatus,
    pub checkpoint: GitCommitCheckpoint,
    /// Generation currently activated for this repository, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_generation_key: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Provider connection metadata owned by one group.
///
/// Read credentials and webhook signing secrets live in the internal secret
/// store; this contract only reports whether each is configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitProviderConnection {
    /// Key of the owning group; connections have no owner-less form.
    pub group_key: String,
    /// Current full path of the owning group.
    pub group_path: String,
    /// Current visibility of the owning group.
    pub visibility: Visibility,
    pub connection_key: String,
    pub provider: GitProviderKind,
    pub mode: GitConnectionMode,
    pub display_name: String,
    pub base_url: String,
    pub has_read_credential: bool,
    pub has_webhook_secret: bool,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Offline readiness of one Git provider connection.
///
/// Readiness is a projection of persisted, non-secret metadata only. It never
/// reflects a provider call, an installation identity, secret-store access, or
/// a secret value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitConnectionReadiness {
    /// Public connections read without a credential.
    Public,
    /// Token connections with a stored read-credential reference.
    Token,
    /// Installation connections whose identity is available. Reserved for the
    /// installation phase; installation connections report `Incomplete` until
    /// an installation identity is persisted.
    Installation,
    /// The connection is missing the state its mode requires.
    Incomplete,
    /// The connection is disabled.
    Disabled,
}

impl GitConnectionReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Token => "token",
            Self::Installation => "installation",
            Self::Incomplete => "incomplete",
            Self::Disabled => "disabled",
        }
    }
}

/// Readiness projection of one group-owned Git provider connection.
///
/// It reports only the non-secret readiness of the connection: the key, its
/// mode, the derived readiness, and the two raw facts the derivation consumes.
/// Provider identity, App private keys, secret-store keys, and secret values
/// are never part of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitConnectionReadinessResponse {
    pub connection_key: String,
    pub mode: GitConnectionMode,
    pub readiness: GitConnectionReadiness,
    pub has_read_credential: bool,
    pub disabled: bool,
}

/// Durable metadata for one index generation of a repository source.
///
/// A generation is metadata only: counts and provenance, never file, blob, or
/// chunk content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitRepositoryGeneration {
    pub generation_key: Uuid,
    pub repository_key: Uuid,
    /// Per-repository monotonic sequence, starting at 1.
    pub generation_number: i64,
    pub ref_name: String,
    /// Pinned snapshot commit this generation covers.
    pub commit_sha: String,
    pub index_profile: GitIndexProfile,
    pub status: GitGenerationStatus,
    /// File entries the generation covers.
    pub file_count: i64,
    /// File entries acquisition excluded, so coverage gaps stay visible.
    pub excluded_file_count: i64,
    pub total_bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The generation a repository source currently serves from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitActiveGeneration {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub activated_at: DateTime<Utc>,
}

/// Whether the integration owns the repository webhook it registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitWebhookOwnership {
    /// Created and managed by context69; safe to update or remove.
    Integration,
    /// Pre-existing external hook the integration must not manage.
    External,
    /// Ownership has not been established.
    Unknown,
}

impl GitWebhookOwnership {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Integration => "integration",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }
}

/// A webhook registration owned by (or observed for) a Git repository source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitWebhookRegistration {
    pub repository_key: Uuid,
    pub provider: GitProviderKind,
    pub external_hook_id: String,
    pub ownership: GitWebhookOwnership,
    pub active: bool,
    pub has_signing_secret: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Delivery idempotency state of a received provider webhook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitWebhookDeliveryStatus {
    Received,
    Queued,
    Ignored,
    Failed,
}

impl GitWebhookDeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Queued => "queued",
            Self::Ignored => "ignored",
            Self::Failed => "failed",
        }
    }
}

/// One provider webhook delivery keyed by its provider delivery id, so a
/// redelivery is recognised instead of enqueued twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GitWebhookDelivery {
    pub delivery_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_key: Option<Uuid>,
    pub provider: GitProviderKind,
    pub status: GitWebhookDeliveryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_commit_sha: Option<String>,
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<DateTime<Utc>>,
}
