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
    GitProviderKind, GitRefreshPolicy, GitRepositorySource, GitVersionPolicy, GitWebhookDelivery,
    GitWebhookDeliveryStatus, GitWebhookOwnership, GitWebhookRegistration,
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
            disabled_at: row.disabled_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

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
    pub target_commit_sha: Option<String>,
    pub index_profile: GitIndexProfile,
    pub refresh_policy: GitRefreshPolicy,
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
    pub refresh: GitRefreshPolicy,
    pub index_profile: GitIndexProfile,
    pub index_status: GitIndexStatus,
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
                commit_sha: row.target_commit_sha.clone(),
            },
            refresh: enums::refresh_policy(&row.refresh_policy)?,
            index_profile: enums::index_profile(&row.index_profile)?,
            index_status: enums::index_status(&row.index_status)?,
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
            refresh: self.refresh,
            index_profile: self.index_profile,
            index_status: self.index_status,
            checkpoint: self.checkpoint.clone(),
            active_generation_key: self.active_generation_key,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Commit checkpoint update for incremental sync.
#[derive(Debug, Clone)]
pub struct GitCheckpointUpdate {
    pub target_commit_sha: Option<String>,
    pub indexed_commit_sha: Option<String>,
    pub index_status: GitIndexStatus,
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
    pub provider: GitProviderKind,
    pub repository_key: Option<Uuid>,
    pub status: GitWebhookDeliveryStatus,
    pub target_commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGitWebhookDelivery {
    pub delivery_id: String,
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
