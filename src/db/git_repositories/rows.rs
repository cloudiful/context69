//! Database row shapes for Git persistence (issue #681 work unit 3B1).
//!
//! Rows are private to the module: every stored struct is built from one of
//! these through a `from_row` constructor, so a query can never hand a
//! half-populated record to a caller. Group columns (`group_id`, `group_key`,
//! `group_path`, `group_visibility`) are selected by the same query so
//! ownership and the group's current visibility travel with the row instead of
//! being snapshotted at write time.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitProviderConnectionRow {
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub group_visibility: String,
    pub connection_key: String,
    pub provider_kind: String,
    pub connection_mode: String,
    pub display_name: String,
    pub base_url: String,
    pub credential_secret_key: Option<String>,
    pub webhook_secret_key: Option<String>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitRepositorySourceRow {
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub group_visibility: String,
    pub repository_key: Uuid,
    pub connection_key: Option<String>,
    pub provider_kind: String,
    pub canonical_url: String,
    pub repository_owner: String,
    pub repository_name: String,
    pub default_branch: String,
    pub target_ref: String,
    pub target_commit_sha: Option<String>,
    pub indexed_commit_sha: Option<String>,
    pub index_profile: String,
    pub refresh_policy: String,
    pub index_status: String,
    pub last_indexed_at: Option<DateTime<Utc>>,
    pub checkpoint_updated_at: Option<DateTime<Utc>>,
    pub active_generation_key: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitRepositoryGenerationRow {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    pub ref_name: String,
    pub commit_sha: String,
    pub index_profile: String,
    pub status: String,
    pub file_count: i64,
    pub excluded_file_count: i64,
    pub total_bytes: i64,
    pub error_code: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitActiveGenerationRow {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub activated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitWebhookRegistrationRow {
    pub repository_key: Uuid,
    pub provider_kind: String,
    pub external_hook_id: String,
    pub ownership: String,
    pub active: bool,
    pub signing_secret_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct GitWebhookDeliveryRow {
    pub delivery_id: String,
    pub provider_kind: String,
    pub repository_key: Option<Uuid>,
    pub status: String,
    pub target_commit_sha: Option<String>,
    pub received_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}
