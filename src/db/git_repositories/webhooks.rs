//! Git webhook registration and delivery persistence (issue #681 work unit
//! 3B1).
//!
//! Registrations hang off a repository source, so they are group-scoped
//! through that join. Deliveries are keyed by the provider delivery id and are
//! recorded before a repository is necessarily known, so they stay
//! provider-scoped; group resolution happens when a delivery is dispatched.

use anyhow::Result;
use uuid::Uuid;

use super::rows::{GitWebhookDeliveryRow, GitWebhookRegistrationRow};
use super::types::{
    NewGitWebhookDelivery, NewGitWebhookRegistration, StoredGitWebhookDelivery,
    StoredGitWebhookRegistration,
};
use crate::contracts::sources::GitWebhookDeliveryStatus;
use crate::db::Database;

impl Database {
    /// Registers or updates the webhook of a repository source owned by
    /// `group_id`, reporting an error when the source belongs to another group.
    pub async fn upsert_git_webhook_registration(
        &self,
        group_id: i64,
        registration: &NewGitWebhookRegistration,
    ) -> Result<StoredGitWebhookRegistration> {
        let row = sqlx::query_file_as!(
            GitWebhookRegistrationRow,
            "src/sql/db/git_repositories/upsert_git_webhook_registration.sql",
            registration.repository_key,
            group_id,
            registration.provider.as_str(),
            registration.external_hook_id,
            registration.ownership.as_str(),
            registration.active,
            registration.signing_secret_key
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("no git repository source of this group to attach a webhook to")
        })?;
        StoredGitWebhookRegistration::from_row(row)
    }

    /// Reads the webhook registration of a repository source owned by
    /// `group_id`; a source of another group reads as absent.
    pub async fn get_git_webhook_registration(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitWebhookRegistration>> {
        let row = sqlx::query_file_as!(
            GitWebhookRegistrationRow,
            "src/sql/db/git_repositories/get_git_webhook_registration.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitWebhookRegistration::from_row).transpose()
    }

    /// Records a provider delivery, returning the stored row. A redelivery of
    /// the same `delivery_id` inserts nothing and returns the original row, so
    /// callers can compare `received_at` to detect the duplicate.
    pub async fn record_git_webhook_delivery(
        &self,
        delivery: &NewGitWebhookDelivery,
    ) -> Result<StoredGitWebhookDelivery> {
        let row = sqlx::query_file_as!(
            GitWebhookDeliveryRow,
            "src/sql/db/git_repositories/record_git_webhook_delivery.sql",
            delivery.delivery_id,
            delivery.provider.as_str(),
            delivery.repository_key,
            delivery.status.as_str(),
            delivery.target_commit_sha
        )
        .fetch_one(&self.pool)
        .await?;
        StoredGitWebhookDelivery::from_row(row)
    }

    pub async fn mark_git_webhook_delivery_processed(
        &self,
        delivery_id: &str,
        status: GitWebhookDeliveryStatus,
        target_commit_sha: Option<&str>,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_webhook_delivery_processed.sql",
            delivery_id,
            status.as_str(),
            target_commit_sha
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }
}
