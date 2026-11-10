//! Stored and insert-side types for Git persistence (issue #681 work unit
//! 3B1).
//!
//! Every stored type is constructed from a database row, so the only way to
//! obtain one is through a successful query, and every stored type knows its own
//! contract projection.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::Visibility;
use crate::contracts::sources::{
    GitCommitCheckpoint, GitConnectionMode, GitIndexProfile, GitIndexStatus, GitProviderConnection,
    GitProviderKind, GitRepositorySkipSummary, GitRepositorySource, GitVersionPolicy,
    GitWebhookDelivery, GitWebhookDeliveryStatus, GitWebhookOwnership, GitWebhookRegistration,
};

use super::enums;
use super::rows::{
    GitProviderConnectionRow, GitRepositorySourceRow, GitWebhookDeliveryRow,
    GitWebhookRegistrationRow,
};

/// Group ownership plus the owning group's current path and visibility.
///
/// `group_id` is the mandatory owner; `group_key`, `group_path`, and
/// `visibility` are read from `context69.groups` on every query, so a group
/// rename or visibility change is reflected immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitGroupOwnership {
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
}

/// Provider connection metadata to insert or update.
#[derive(Debug, Clone)]
pub struct NewGitProviderConnection {
    pub connection_key: String,
    pub provider: GitProviderKind,
    pub mode: GitConnectionMode,
    pub display_name: String,
    pub base_url: String,
    /// Internal secret-store key for the read credential; never the value.
    pub credential_secret_key: Option<String>,
    /// Internal secret-store key for the webhook signing secret; never the value.
    pub webhook_secret_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitProviderConnection {
    pub group: GitGroupOwnership,
    pub connection_key: String,
    pub provider: GitProviderKind,
    pub mode: GitConnectionMode,
    pub display_name: String,
    pub base_url: String,
    pub credential_secret_key: Option<String>,
    pub webhook_secret_key: Option<String>,
    /// Internal secret-store key for the GitHub App private key; never the
    /// value. Read from the row only: the insert side cannot set it, so the
    /// narrow App-key reference statement is the one way it moves.
    pub app_private_key_secret_key: Option<String>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StoredGitProviderConnection {
    pub(crate) fn from_row(row: GitProviderConnectionRow) -> Result<Self> {
        Ok(Self {
            group: GitGroupOwnership {
                group_id: row.group_id,
                group_key: row.group_key,
                group_path: row.group_path,
                visibility: enums::visibility(&row.group_visibility)?,
            },
            connection_key: row.connection_key,
            provider: enums::provider_kind(&row.provider_kind)?,
            mode: enums::connection_mode(&row.connection_mode)?,
            display_name: row.display_name,
            base_url: row.base_url,
            credential_secret_key: row.credential_secret_key,
            webhook_secret_key: row.webhook_secret_key,
            app_private_key_secret_key: row.app_private_key_secret_key,
            disabled_at: row.disabled_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// The outward projection.
    ///
    /// Purpose separation keeps the GitHub App private key out of it: the
    /// contract reports which secrets are configured through the two existing
    /// flags, and this phase adds no third boolean for the App key.
    pub fn to_contract(&self) -> GitProviderConnection {
        GitProviderConnection {
            group_key: self.group.group_key.clone(),
            group_path: self.group.group_path.clone(),
            visibility: self.group.visibility,
            connection_key: self.connection_key.clone(),
            provider: self.provider,
            mode: self.mode,
            display_name: self.display_name.clone(),
            base_url: self.base_url.clone(),
            has_read_credential: self.credential_secret_key.is_some(),
            has_webhook_secret: self.webhook_secret_key.is_some(),
            disabled: self.disabled_at.is_some(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// A Git repository source registration to insert or update.
#[derive(Debug, Clone)]
pub struct NewGitRepositorySource {
    pub connection_key: Option<String>,
    pub provider: GitProviderKind,
    pub canonical_url: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub target_ref: String,
    pub pinned_commit_sha: Option<String>,
    pub index_profile: GitIndexProfile,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub interval_seconds: Option<i64>,
    pub webhook_enabled: bool,
    pub enabled: bool,
    pub retention_count: i64,
    /// Manager whose registration created this source, when one is known.
    pub created_by_user_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitRepositorySource {
    pub group: GitGroupOwnership,
    pub repository_key: Uuid,
    pub connection_key: Option<String>,
    pub provider: GitProviderKind,
    pub canonical_url: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub version: GitVersionPolicy,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub interval_seconds: Option<i64>,
    pub webhook_enabled: bool,
    pub enabled: bool,
    pub retention_count: i64,
    /// Manager whose registration created this source, when one is known.
    pub created_by_user_id: Option<i64>,
    /// Monotonic fence for edits that change what a snapshot means.
    pub config_revision: i64,
    /// A refresh was requested since the last completed snapshot.
    pub refresh_dirty: bool,
    /// When the latest refresh request arrived.
    pub refresh_requested_at: Option<DateTime<Utc>>,
    /// When a refresh attempt was last admitted, so retries stay bounded.
    pub last_refresh_attempt_at: Option<DateTime<Utc>>,
    /// A force rebuild demand that arrived while another run was active.
    pub force_rebuild_requested_at: Option<DateTime<Utc>>,
    /// Set when a delete request could not finish vector cleanup.
    pub deletion_requested_at: Option<DateTime<Utc>>,
    pub last_skip_summary: GitRepositorySkipSummary,
    /// Pipeline version that produced the active generation.
    pub indexed_pipeline_version: u32,
    /// Configuration fingerprint that produced the active generation.
    pub indexed_config_fingerprint: String,
    pub index_profile: GitIndexProfile,
    pub index_status: GitIndexStatus,
    /// Bounded stable code of the last terminal failure, when one is known.
    pub last_error_code: Option<String>,
    pub checkpoint: GitCommitCheckpoint,
    /// Generation currently serving this repository, when one is activated.
    pub active_generation_key: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StoredGitRepositorySource {
    pub(crate) fn from_row(row: GitRepositorySourceRow) -> Result<Self> {
        Ok(Self {
            group: GitGroupOwnership {
                group_id: row.group_id,
                group_key: row.group_key,
                group_path: row.group_path,
                visibility: enums::visibility(&row.group_visibility)?,
            },
            repository_key: row.repository_key,
            connection_key: row.connection_key,
            provider: enums::provider_kind(&row.provider_kind)?,
            canonical_url: row.canonical_url,
            owner: row.repository_owner,
            name: row.repository_name,
            default_branch: row.default_branch,
            version: GitVersionPolicy {
                ref_name: row.target_ref,
                commit_sha: row.pinned_commit_sha,
            },
            include_patterns: row.include_patterns,
            exclude_patterns: row.exclude_patterns,
            interval_seconds: row.interval_seconds,
            webhook_enabled: row.webhook_enabled,
            enabled: row.enabled,
            retention_count: row.retention_count,
            created_by_user_id: row.created_by_user_id,
            config_revision: row.config_revision,
            refresh_dirty: row.refresh_dirty,
            refresh_requested_at: row.refresh_requested_at,
            last_refresh_attempt_at: row.last_refresh_attempt_at,
            force_rebuild_requested_at: row.force_rebuild_requested_at,
            deletion_requested_at: row.deletion_requested_at,
            last_skip_summary: skip_summary(&row.last_skip_summary),
            indexed_pipeline_version: u32::try_from(row.indexed_pipeline_version)
                .unwrap_or_default(),
            indexed_config_fingerprint: row.indexed_config_fingerprint,
            index_profile: enums::index_profile(&row.index_profile)?,
            index_status: enums::index_status(&row.index_status)?,
            last_error_code: row.last_error_code,
            checkpoint: GitCommitCheckpoint {
                target_commit_sha: row.target_commit_sha,
                indexed_commit_sha: row.indexed_commit_sha,
                indexed_at: row.last_indexed_at,
                checkpoint_updated_at: row.checkpoint_updated_at,
            },
            active_generation_key: row.active_generation_key,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub fn to_contract(&self) -> GitRepositorySource {
        GitRepositorySource {
            group_key: self.group.group_key.clone(),
            group_path: self.group.group_path.clone(),
            visibility: self.group.visibility,
            repository_key: self.repository_key,
            connection_key: self.connection_key.clone(),
            provider: self.provider,
            canonical_url: self.canonical_url.clone(),
            owner: self.owner.clone(),
            name: self.name.clone(),
            default_branch: self.default_branch.clone(),
            version: self.version.clone(),
            include_patterns: self.include_patterns.clone(),
            exclude_patterns: self.exclude_patterns.clone(),
            interval_seconds: self.interval_seconds,
            webhook_enabled: self.webhook_enabled,
            enabled: self.enabled,
            retention_count: self.retention_count,
            refresh_pending: self.refresh_dirty,
            last_refresh_attempt_at: self.last_refresh_attempt_at,
            deletion_pending: self.deletion_requested_at.is_some(),
            last_skip_summary: self.last_skip_summary,
            index_profile: self.index_profile,
            index_status: self.index_status,
            last_error_code: self.last_error_code.clone(),
            checkpoint: self.checkpoint.clone(),
            active_generation_key: self.active_generation_key,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Decode the stored skip summary, defaulting to empty rather than failing a
/// read when an older row predates the field.
fn skip_summary(value: &serde_json::Value) -> GitRepositorySkipSummary {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// Encode the skip summary for the stored JSONB column.
pub(crate) fn skip_summary_value(summary: GitRepositorySkipSummary) -> serde_json::Value {
    serde_json::to_value(summary).unwrap_or_else(|_| serde_json::json!({}))
}

/// Editable target, profile, and settings passed to the source update.
#[derive(Debug, Clone)]
pub struct GitRepositorySettingsUpdate<'a> {
    pub target_ref: Option<&'a str>,
    pub pinned_commit_action: &'a str,
    pub pinned_commit_value: Option<&'a str>,
    pub index_profile: Option<&'a str>,
    pub include_patterns: &'a [String],
    pub exclude_patterns: &'a [String],
    pub interval_seconds: Option<i64>,
    pub webhook_enabled: bool,
    pub enabled: bool,
    pub retention_count: i64,
}

/// Commit checkpoint update for incremental sync.
#[derive(Debug, Clone)]
pub struct GitCheckpointUpdate {
    pub target_commit_sha: Option<String>,
    pub indexed_commit_sha: Option<String>,
    pub index_status: GitIndexStatus,
    /// Pipeline version that produced the indexed snapshot, when this update
    /// records a completed generation.
    pub pipeline_version: Option<u32>,
    /// Configuration fingerprint that produced the indexed snapshot.
    pub config_fingerprint: Option<String>,
    /// Skip counts of the indexed snapshot.
    pub skip_summary: Option<GitRepositorySkipSummary>,
}

/// A webhook registration to insert or update for a repository source.
#[derive(Debug, Clone)]
pub struct NewGitWebhookRegistration {
    pub repository_key: Uuid,
    pub provider: GitProviderKind,
    pub external_hook_id: String,
    pub ownership: GitWebhookOwnership,
    pub active: bool,
    /// Internal secret-store key for the signing secret; never the value.
    pub signing_secret_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitWebhookRegistration {
    pub repository_key: Uuid,
    pub provider: GitProviderKind,
    pub external_hook_id: String,
    pub ownership: GitWebhookOwnership,
    pub active: bool,
    pub signing_secret_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StoredGitWebhookRegistration {
    pub(crate) fn from_row(row: GitWebhookRegistrationRow) -> Result<Self> {
        Ok(Self {
            repository_key: row.repository_key,
            provider: enums::provider_kind(&row.provider_kind)?,
            external_hook_id: row.external_hook_id,
            ownership: enums::webhook_ownership(&row.ownership)?,
            active: row.active,
            signing_secret_key: row.signing_secret_key,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub fn to_contract(&self) -> GitWebhookRegistration {
        GitWebhookRegistration {
            repository_key: self.repository_key,
            provider: self.provider,
            external_hook_id: self.external_hook_id.clone(),
            ownership: self.ownership,
            active: self.active,
            has_signing_secret: self.signing_secret_key.is_some(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// A provider webhook delivery to record for idempotency.
#[derive(Debug, Clone)]
pub struct NewGitWebhookDelivery {
    pub delivery_id: String,
    pub external_hook_id: String,
    pub provider: GitProviderKind,
    pub repository_key: Option<Uuid>,
    pub status: GitWebhookDeliveryStatus,
    pub target_commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitWebhookDelivery {
    pub delivery_id: String,
    pub external_hook_id: String,
    pub provider: GitProviderKind,
    pub repository_key: Option<Uuid>,
    pub status: GitWebhookDeliveryStatus,
    pub target_commit_sha: Option<String>,
    pub received_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}

impl StoredGitWebhookDelivery {
    pub(crate) fn from_row(row: GitWebhookDeliveryRow) -> Result<Self> {
        Ok(Self {
            delivery_id: row.delivery_id,
            external_hook_id: row.external_hook_id,
            provider: enums::provider_kind(&row.provider_kind)?,
            repository_key: row.repository_key,
            status: enums::delivery_status(&row.status)?,
            target_commit_sha: row.target_commit_sha,
            received_at: row.received_at,
            processed_at: row.processed_at,
        })
    }

    pub fn to_contract(&self) -> GitWebhookDelivery {
        GitWebhookDelivery {
            delivery_id: self.delivery_id.clone(),
            repository_key: self.repository_key,
            provider: self.provider,
            status: self.status,
            target_commit_sha: self.target_commit_sha.clone(),
            received_at: self.received_at,
            processed_at: self.processed_at,
        }
    }
}
