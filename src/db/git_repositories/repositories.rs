//! Group-scoped repository source persistence (issue #681 work unit 3B1).
//!
//! Identity is per group: two groups may register the same canonical URL and
//! ref, and each group sees only its own sources. Reads, the checkpoint update,
//! and the upsert all filter on the owning group.

use anyhow::Result;
use uuid::Uuid;

use super::rows::GitRepositorySourceRow;
use super::types::{GitCheckpointUpdate, NewGitRepositorySource, StoredGitRepositorySource};
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
            source.target_commit_sha,
            source.index_profile.as_str(),
            source.refresh_policy.as_str()
        )
        .fetch_one(&self.pool)
        .await?;
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
        let row = sqlx::query_file_as!(
            GitRepositorySourceRow,
            "src/sql/db/git_repositories/update_git_repository_checkpoint.sql",
            group_id,
            repository_key,
            checkpoint.target_commit_sha,
            checkpoint.indexed_commit_sha,
            checkpoint.index_status.as_str()
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitRepositorySource::from_row).transpose()
    }
}
