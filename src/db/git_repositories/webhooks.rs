//! Git webhook registration and delivery persistence (issue #681 work unit
//! 3B1).
//!
//! Registrations hang off a repository source, so they are group-scoped
//! through that join. Deliveries are scoped by the receiving registration:
//! provider, provider-issued hook id, and owning repository source. The
//! provider delivery id is only unique inside that scope, so two registrations
//! can receive the same id without sharing or mutating each other's row.
//!
//! An unauthenticated provider ingress knows only the provider and the hook id,
//! so one read resolves a registration by that unique pair and returns the
//! owning group id beside it — the minimum an ingress needs to derive the
//! record-scoped signing-secret name without trusting a caller-supplied group.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::rows::{GitWebhookDeliveryRow, GitWebhookRegistrationRow};
use super::types::{
    NewGitWebhookDelivery, NewGitWebhookRegistration, StoredGitWebhookDelivery,
    StoredGitWebhookRegistration,
};
use crate::contracts::sources::{GitProviderKind, GitWebhookDeliveryStatus};
use crate::db::Database;

/// The columns of a registration read by provider and external hook id, plus
/// the owning group id. The group id is not part of
/// [`StoredGitWebhookRegistration`] because a group-scoped caller already knows
/// it; an ingress that resolves the hook from its provider-facing id does not,
/// and needs it to derive the record-scoped signing-secret name.
#[derive(Debug, sqlx::FromRow)]
struct GitWebhookRegistrationByHookRow {
    group_id: i64,
    repository_key: Uuid,
    provider_kind: String,
    external_hook_id: String,
    ownership: String,
    active: bool,
    signing_secret_key: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Database {
    /// Begins the transaction that serializes registration creation for one
    /// repository of `group_id`.
    ///
    /// Takes a transaction-scoped advisory lock keyed by the owning group and the
    /// repository the registration hangs from as its own statement, so two
    /// concurrent creates for the same repository queue behind each other. The
    /// caller must run the group-scoped pre-check, the seal-only secret write, and
    /// the create-only insert while holding it, then
    /// [`GitWebhookRegistrationCreation::commit`] or
    /// [`GitWebhookRegistrationCreation::rollback`] explicitly: the lock is
    /// released only when that transaction ends.
    ///
    /// Serializing the whole create is what bounds the deterministic-secret race
    /// the seal split would otherwise leave open: two creates for one repository
    /// cannot both seal before either inserts, so the losing request blocks before
    /// it can overwrite the sealed row the winner references, and the value the
    /// winner stored is the winner's own.
    pub async fn begin_git_webhook_registration_creation(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<GitWebhookRegistrationCreation<'static>> {
        let mut tx = self.pool.begin().await?;
        let lock_key =
            format!("context69.git_webhook_registration_creation:{group_id}:{repository_key}");
        sqlx::query_file!(
            "src/sql/db/git_repositories/acquire_git_webhook_registration_creation_lock.sql",
            lock_key
        )
        .execute(&mut *tx)
        .await?;
        Ok(GitWebhookRegistrationCreation { tx })
    }

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
        get_git_webhook_registration_on(&self.pool, group_id, repository_key).await
    }

    /// Resolves a webhook registration by its provider-facing identity, plus
    /// the owning group id.
    ///
    /// The unique `(provider_kind, external_hook_id)` index makes this zero or
    /// one row, and no group is part of the lookup: a caller that only holds the
    /// provider's hook id cannot learn which group owns it except by presenting
    /// the signature the record's secret verifies.
    pub async fn get_git_webhook_registration_by_hook(
        &self,
        provider: GitProviderKind,
        external_hook_id: &str,
    ) -> Result<Option<(i64, StoredGitWebhookRegistration)>> {
        let row = sqlx::query_file_as!(
            GitWebhookRegistrationByHookRow,
            "src/sql/db/git_repositories/get_git_webhook_registration_by_hook.sql",
            provider.as_str(),
            external_hook_id
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let registration = StoredGitWebhookRegistration::from_row(GitWebhookRegistrationRow {
                repository_key: row.repository_key,
                provider_kind: row.provider_kind,
                external_hook_id: row.external_hook_id,
                ownership: row.ownership,
                active: row.active,
                signing_secret_key: row.signing_secret_key,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })?;
            Ok((row.group_id, registration))
        })
        .transpose()
    }

    /// Narrows the signing-secret reference of one webhook registration owned by
    /// `group_id`, reporting whether a row matched.
    ///
    /// Group ownership is resolved through the registration's repository source,
    /// so a registration of another group's repository matches no row. `active`
    /// is preserved: sealing a signing secret is not a lifecycle change.
    pub async fn set_git_webhook_signing_secret_key(
        &self,
        group_id: i64,
        repository_key: Uuid,
        secret_key: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/git_repositories/set_git_webhook_signing_secret_key.sql",
            group_id,
            repository_key,
            secret_key
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Records a provider delivery, returning the stored row. A redelivery of
    /// the same id inside the same registration inserts nothing and returns the
    /// original row, so callers can compare `received_at` to detect the
    /// duplicate. Another registration's identical delivery id inserts its own
    /// row.
    pub async fn record_git_webhook_delivery(
        &self,
        delivery: &NewGitWebhookDelivery,
    ) -> Result<StoredGitWebhookDelivery> {
        let row = sqlx::query_file_as!(
            GitWebhookDeliveryRow,
            "src/sql/db/git_repositories/record_git_webhook_delivery.sql",
            delivery.delivery_id,
            delivery.external_hook_id,
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
        provider: GitProviderKind,
        external_hook_id: &str,
        repository_key: Uuid,
        delivery_id: &str,
        status: GitWebhookDeliveryStatus,
        target_commit_sha: Option<&str>,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_webhook_delivery_processed.sql",
            provider.as_str(),
            external_hook_id,
            repository_key,
            delivery_id,
            status.as_str(),
            target_commit_sha
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }
}

/// The transaction that serializes creation of one registration.
///
/// The caller never names this type: it is the value
/// [`Database::begin_git_webhook_registration_creation`] returns, and the lock it
/// holds is released only by the explicit [`Self::commit`] or [`Self::rollback`].
pub struct GitWebhookRegistrationCreation<'a> {
    tx: Transaction<'a, Postgres>,
}

impl GitWebhookRegistrationCreation<'_> {
    /// Reads the group-owned registration inside the locked transaction, so the
    /// pre-check sees a winner that committed before the lock was taken.
    pub async fn get(
        &mut self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitWebhookRegistration>> {
        get_git_webhook_registration_on(&mut *self.tx, group_id, repository_key).await
    }

    /// Runs the create-only insert inside the locked transaction.
    ///
    /// The insert has no conflict clause, so a repository that already has a
    /// registration and a hook identity another repository already claims both
    /// raise their unique violation for the caller to map to one bounded conflict.
    pub async fn insert(
        &mut self,
        group_id: i64,
        registration: &NewGitWebhookRegistration,
    ) -> Result<StoredGitWebhookRegistration> {
        insert_git_webhook_registration_on(&mut *self.tx, group_id, registration).await
    }

    /// Commits and releases the lock.
    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Rolls back and releases the lock.
    pub async fn rollback(self) -> Result<()> {
        self.tx.rollback().await?;
        Ok(())
    }
}

/// Reads one registration through any executor, so the plain pool read and the
/// locked transaction read share one statement and one shape.
async fn get_git_webhook_registration_on<'e, E>(
    executor: E,
    group_id: i64,
    repository_key: Uuid,
) -> Result<Option<StoredGitWebhookRegistration>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_file_as!(
        GitWebhookRegistrationRow,
        "src/sql/db/git_repositories/get_git_webhook_registration.sql",
        group_id,
        repository_key
    )
    .fetch_optional(executor)
    .await?;
    row.map(StoredGitWebhookRegistration::from_row).transpose()
}

/// Runs the create-only insert through any executor, so the locked transaction is
/// the only way the create path writes a registration.
async fn insert_git_webhook_registration_on<'e, E>(
    executor: E,
    group_id: i64,
    registration: &NewGitWebhookRegistration,
) -> Result<StoredGitWebhookRegistration>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_file_as!(
        GitWebhookRegistrationRow,
        "src/sql/db/git_repositories/insert_git_webhook_registration.sql",
        registration.repository_key,
        group_id,
        registration.provider.as_str(),
        registration.external_hook_id,
        registration.ownership.as_str(),
        registration.active,
        registration.signing_secret_key
    )
    .fetch_one(executor)
    .await?;
    StoredGitWebhookRegistration::from_row(row)
}
