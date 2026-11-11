//! Group-scoped repository source persistence (issue #681 work unit 3B1).
//!
//! Identity is per group: two groups may register the same canonical URL and
//! ref, and each group sees only its own sources. Reads, the checkpoint update,
//! and the upsert all filter on the owning group.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::rows::GitRepositorySourceRow;
use super::types::{
    GitCheckpointUpdate, GitRepositorySettingsUpdate, NewGitRepositorySource,
    StoredGitRepositorySource,
};
use crate::db::Database;

impl Database {
    /// Registers or updates a repository source owned by `group_id`.
    ///
    /// The conflict target is `(group_id, canonical_url, target_ref)`, so an
    /// upsert updates the calling group's own registration and cannot create or
    /// take over another group's registration of the same repository ref.
    pub async fn upsert_git_repository_source(
        &self,
        group_id: i64,
        source: &NewGitRepositorySource,
    ) -> Result<StoredGitRepositorySource> {
        // The shared ancestor-key lock and the insert share one transaction, so
        // a concurrent delete of this group or any ancestor cannot cascade the
        // group between the ownership check and this registration. See
        // `lock_git_group_source_registration.sql`.
        let mut tx = self.pool.begin().await?;
        sqlx::query_file!(
            "src/sql/db/git_repositories/lock_git_group_source_registration.sql",
            group_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file_scalar!(
            "src/sql/db/git_repositories/lock_git_group_source_ancestors.sql",
            group_id
        )
        .fetch_all(&mut *tx)
        .await?;
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/upsert_git_repository_source.sql",
            group_id,
            source.connection_key,
            source.provider.as_str(),
            source.canonical_url,
            source.owner,
            source.name,
            source.default_branch,
            source.target_ref,
            source.pinned_commit_sha,
            source.index_profile.as_str(),
            &source.include_patterns,
            &source.exclude_patterns,
            source.interval_seconds,
            source.webhook_enabled,
            source.enabled,
            source.retention_count,
            source.created_by_user_id
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        StoredGitRepositorySource::from_row(row)
    }

    /// Reads one source owned by `group_id`; a source of another group reads as
    /// absent.
    pub async fn get_git_repository_source(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitRepositorySource>> {
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/get_git_repository_source.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitRepositorySource::from_row).transpose()
    }

    /// Lists the sources of one group only.
    pub async fn list_git_repository_sources(
        &self,
        group_id: i64,
    ) -> Result<Vec<StoredGitRepositorySource>> {
        let rows = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/list_git_repository_sources.sql",
            group_id
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(StoredGitRepositorySource::from_row)
            .collect()
    }

    /// Advances the incremental-sync checkpoint of a source owned by
    /// `group_id`, reporting `None` when the source belongs to another group.
    pub async fn update_git_repository_checkpoint(
        &self,
        group_id: i64,
        repository_key: Uuid,
        checkpoint: &GitCheckpointUpdate,
    ) -> Result<Option<StoredGitRepositorySource>> {
        let pipeline_version = checkpoint.pipeline_version.map(i64::from);
        let skip_summary = checkpoint
            .skip_summary
            .map(super::types::skip_summary_value);
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/update_git_repository_checkpoint.sql",
            group_id,
            repository_key,
            checkpoint.target_commit_sha,
            checkpoint.indexed_commit_sha,
            checkpoint.index_status.as_str(),
            pipeline_version,
            checkpoint.config_fingerprint,
            skip_summary
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitRepositorySource::from_row).transpose()
    }

    /// Moves a source to `indexing` and records the resolved target, but only
    /// while the configuration revision and observed target a run read are still
    /// current.
    pub async fn begin_git_repository_index(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
        expected_target_commit_sha: Option<&str>,
        observed_commit_sha: &str,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/begin_git_repository_index.sql",
            group_id,
            repository_key,
            expected_config_revision,
            expected_target_commit_sha,
            observed_commit_sha
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Refreshes the checkpoint of a same-target no-op only while its captured
    /// configuration revision and observed target are still current.
    pub async fn complete_git_repository_noop(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
        expected_target_commit_sha: Option<&str>,
        observed_commit_sha: &str,
        pipeline_version: u32,
        config_fingerprint: &str,
        skip_summary: &crate::contracts::sources::GitRepositorySkipSummary,
    ) -> Result<bool> {
        let skip_summary = super::types::skip_summary_value(*skip_summary);
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/complete_git_repository_noop.sql",
            group_id,
            repository_key,
            expected_config_revision,
            expected_target_commit_sha,
            observed_commit_sha,
            i64::from(pipeline_version),
            config_fingerprint,
            skip_summary
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Marks the source failed only while the attempt's configuration revision
    /// is still current.
    pub async fn fail_git_repository_index(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/fail_git_repository_index.sql",
            group_id,
            repository_key,
            expected_config_revision
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Records one durable refresh demand on an enabled source of `group_id`.
    pub async fn request_git_refresh(&self, group_id: i64, repository_key: Uuid) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/request_git_refresh.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Records a refresh demand inside the caller's submission transaction, so
    /// admitting a reused task and marking the demand commit together.
    pub(crate) async fn request_git_refresh_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/request_git_refresh.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.is_some())
    }

    /// Records a force-rebuild demand on an enabled source.
    pub async fn request_git_force_rebuild(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/request_git_force_rebuild.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Records a force-rebuild demand inside the caller's submission
    /// transaction, alongside the refresh marker that schedules the follow-up.
    pub(crate) async fn request_git_force_rebuild_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/request_git_force_rebuild.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.is_some())
    }

    /// Bounds retries by recording that a refresh attempt was admitted.
    pub async fn mark_git_refresh_attempt(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_refresh_attempt.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Clears a demand only when no newer request arrived after `observed_at`.
    pub async fn consume_git_refresh_request(
        &self,
        group_id: i64,
        repository_key: Uuid,
        observed_at: DateTime<Utc>,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/consume_git_refresh_request.sql",
            group_id,
            repository_key,
            observed_at
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Due refresh sources, bounded by `limit`.
    pub async fn list_due_git_refresh_sources(&self, limit: i64) -> Result<Vec<(i64, Uuid)>> {
        let rows = sqlx::query_file!(
            "src/sql/db/git_repositories/list_due_git_refresh_sources.sql",
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.group_id, row.repository_key))
            .collect())
    }

    /// Sources with a pending delete whose cleanup can be retried.
    pub async fn list_pending_git_deletions(&self, limit: i64) -> Result<Vec<(i64, Uuid)>> {
        let rows = sqlx::query_file!(
            "src/sql/db/git_repositories/list_pending_git_deletions.sql",
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.group_id, row.repository_key))
            .collect())
    }

    /// Marks a source for deletion and stops further admissions.
    pub async fn mark_git_repository_deletion_requested(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_repository_deletion_requested.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Marks a source for deletion inside the caller's admission transaction.
    pub(crate) async fn mark_git_repository_deletion_requested_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_repository_deletion_requested.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.is_some())
    }

    /// Records a failed source-deletion cleanup and its bounded retry time.
    pub(crate) async fn mark_git_repository_deletion_retry(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_repository_deletion_retry.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Removes a source after its vector content is gone.
    pub async fn delete_git_repository_source(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/delete_git_repository_source.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Resolves the user a background refresh is attributed to.
    pub async fn resolve_git_refresh_actor(
        &self,
        group_id: i64,
        preferred_user_id: Option<i64>,
    ) -> Result<Option<i64>> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/resolve_git_refresh_actor.sql",
            group_id,
            preferred_user_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|row| row.user_id))
    }

    /// Replaces the editable target, profile, and settings of one source owned
    /// by `group_id`.
    pub async fn update_git_repository_source_settings(
        &self,
        group_id: i64,
        repository_key: Uuid,
        update: &GitRepositorySettingsUpdate<'_>,
    ) -> Result<Option<StoredGitRepositorySource>> {
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/update_git_repository_source_settings.sql",
            group_id,
            repository_key,
            update.target_ref,
            update.pinned_commit_action,
            update.pinned_commit_value,
            update.index_profile,
            update.include_patterns,
            update.exclude_patterns,
            update.interval_seconds,
            update.webhook_enabled,
            update.enabled,
            update.retention_count
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitRepositorySource::from_row).transpose()
    }

    /// Records the bounded stable code of a terminal index failure only while
    /// the attempt's configuration revision is still current.
    pub async fn mark_git_repository_failure(
        &self,
        group_id: i64,
        repository_key: Uuid,
        expected_config_revision: i64,
        error_code: &str,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/mark_git_repository_failure.sql",
            group_id,
            repository_key,
            expected_config_revision,
            error_code
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }
}

/// One held per-repository index lifetime advisory lock.
///
/// The lock lives in a transaction on its own pooled connection, so a long
/// provider or vector pass never keeps table row locks or a snapshot open. The
/// transaction is only used for the advisory lock; every data statement takes
/// its own connection. Dropping the guard queues the rollback that releases the
/// lock, so cancellation and errors release it too.
pub(crate) struct GitIndexLifetimeGuard {
    tx: Option<Transaction<'static, Postgres>>,
}

impl GitIndexLifetimeGuard {
    /// Waits for the lifetime lock, as an index run must.
    pub(crate) async fn acquire(pool: &PgPool, repository_key: Uuid) -> Result<Self> {
        let mut tx = pool.begin().await?;
        let key = git_index_lifetime_lock_key(repository_key);
        sqlx::query_file!(
            "src/sql/db/git_repositories/lock_git_index_lifetime.sql",
            key
        )
        .execute(&mut *tx)
        .await?;
        Ok(Self { tx: Some(tx) })
    }

    /// Takes the lifetime lock only if it is free, as cleanup and source
    /// deletion must so they can defer instead of blocking an active run.
    pub(crate) async fn try_acquire(pool: &PgPool, repository_key: Uuid) -> Result<Option<Self>> {
        let mut tx = pool.begin().await?;
        let key = git_index_lifetime_lock_key(repository_key);
        let row = sqlx::query_file!(
            "src/sql/db/git_repositories/try_lock_git_index_lifetime.sql",
            key
        )
        .fetch_one(&mut *tx)
        .await?;
        if row.acquired != Some(true) {
            tx.rollback().await?;
            return Ok(None);
        }
        Ok(Some(Self { tx: Some(tx) }))
    }

    /// Releases the lock explicitly before the caller reports its outcome.
    pub(crate) async fn release(mut self) -> Result<()> {
        if let Some(tx) = self.tx.take() {
            tx.commit().await?;
        }
        Ok(())
    }
}

/// The lifetime lock key is separate from the short admission lock so holding
/// one never blocks admitting or observing the other.
fn git_index_lifetime_lock_key(repository_key: Uuid) -> String {
    format!("context69.git_index_lifetime:{repository_key}")
}
