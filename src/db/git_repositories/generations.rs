//! Durable index generation persistence (issue #681 work unit 3B1).
//!
//! A generation is metadata only: the pinned commit it covers and the coverage
//! counters acquisition reported. Content lives in the lexical and vector
//! stores, never here.
//!
//! Completion and activation are one statement: the generation's counters and
//! completion time, the supersession of the previously active generation, and
//! the repository's active-generation pointer either all land or none do.

use anyhow::{Context, Result};
use sqlx::FromRow;
use uuid::Uuid;

use super::generation_types::{
    GitGenerationCleanupTarget, GitGenerationCoverage, NewGitGenerationCodeVectors,
    NewGitRepositoryGeneration, StoredGitActiveGeneration, StoredGitGenerationCodeVectors,
    StoredGitRepositoryGeneration,
};
use super::rows::{
    GitActiveGenerationRow, GitGenerationCodeVectorsRow, GitRepositoryGenerationRow,
};
use crate::db::Database;

#[derive(Debug, FromRow)]
struct GitGenerationCleanupRow {
    generation_key: Uuid,
    status: String,
    cleanup_pending: bool,
    fingerprint: Option<String>,
    endpoint_identity: Option<String>,
    dimensions: Option<i64>,
    collection_name: Option<String>,
    point_count: Option<i64>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl GitGenerationCleanupRow {
    fn into_target(self) -> GitGenerationCleanupTarget {
        let code_vectors = match (
            self.fingerprint,
            self.dimensions,
            self.collection_name,
            self.point_count,
            self.created_at,
        ) {
            (
                Some(fingerprint),
                Some(dimensions),
                Some(collection_name),
                Some(point_count),
                Some(created_at),
            ) => Some(StoredGitGenerationCodeVectors {
                generation_key: self.generation_key,
                fingerprint,
                endpoint_identity: self.endpoint_identity,
                dimensions,
                collection_name,
                point_count,
                created_at,
            }),
            _ => None,
        };
        GitGenerationCleanupTarget::from_row(
            self.generation_key,
            self.status,
            self.cleanup_pending,
            code_vectors,
        )
    }
}

impl Database {
    /// Opens the next generation of a repository owned by `group_id`, pinned to
    /// the given ref and commit.
    ///
    /// The generation number is allocated by incrementing the repository's own
    /// counter in the same statement that inserts the generation. The
    /// increment applies to the row version the previous holder committed, so
    /// concurrent starts take distinct consecutive numbers rather than both
    /// deriving the same one from a stale read, and no retry is needed.
    /// Allocation and insertion share a transaction, so a start that fails
    /// consumes no number. A repository of another group matches no row, which
    /// surfaces as an error.
    pub async fn start_git_repository_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation: &NewGitRepositoryGeneration,
    ) -> Result<StoredGitRepositoryGeneration> {
        let row = sqlx::query_file_as!(
            GitRepositoryGenerationRow,
            "src/sql/db/git_repository_generations/start_git_repository_generation.sql",
            group_id,
            repository_key,
            generation.ref_name,
            generation.commit_sha,
            generation.index_profile.as_str()
        )
        .fetch_optional(&self.pool)
        .await?
        .context("no git repository source of this group to index")?;
        StoredGitRepositoryGeneration::from_row(row)
    }

    /// Completes a building generation and activates it for its repository in a
    /// single atomic database operation.
    ///
    /// Returns an error, changing nothing, when the generation does not belong
    /// to a repository of `group_id` or is not building: an already completed,
    /// failed, or foreign generation can never be activated or rewritten.
    pub async fn complete_and_activate_git_repository_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        coverage: GitGenerationCoverage,
    ) -> Result<StoredGitActiveGeneration> {
        let row = sqlx::query_file_as!(
            GitActiveGenerationRow,
            "src/sql/db/git_repository_generations/complete_and_activate_git_repository_generation.sql",
            group_id,
            repository_key,
            generation_key,
            coverage.file_count,
            coverage.excluded_file_count,
            coverage.total_bytes
        )
        .fetch_optional(&self.pool)
        .await?
        .context(
            "no building git index generation of this group and repository to complete",
        )?;
        StoredGitActiveGeneration::from_row(row)
    }

    /// Publishes a building generation and advances the source checkpoint in a
    /// single atomic operation, only while the attempt's configuration revision
    /// and observed target are still current.
    ///
    /// Returns `None`, changing nothing, when a newer source edit or observed
    /// target owns the repository. A force-rebuild demand is consumed only when
    /// it is not newer than the demand this attempt observed.
    pub async fn publish_git_repository_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        publication: &super::generation_types::GitGenerationPublication,
    ) -> Result<Option<StoredGitActiveGeneration>> {
        let skip_summary = super::types::skip_summary_value(publication.skip_summary);
        let row = sqlx::query_file_as!(
            GitActiveGenerationRow,
            "src/sql/db/git_repository_generations/publish_git_repository_generation.sql",
            group_id,
            repository_key,
            generation_key,
            publication.expected_config_revision,
            publication.observed_commit_sha,
            i64::from(publication.pipeline_version),
            publication.config_fingerprint,
            skip_summary,
            publication.coverage.file_count,
            publication.coverage.excluded_file_count,
            publication.coverage.total_bytes,
            publication.observed_force_rebuild_requested_at
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitActiveGeneration::from_row).transpose()
    }

    /// Marks a building generation failed with a bounded error code, reporting
    /// whether a row matched. A failed generation is never activated.
    pub async fn fail_git_repository_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        error_code: &str,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repository_generations/fail_git_repository_generation.sql",
            group_id,
            repository_key,
            generation_key,
            error_code
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Reads one generation of a repository owned by `group_id`.
    pub async fn get_git_repository_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
    ) -> Result<Option<StoredGitRepositoryGeneration>> {
        let row = sqlx::query_file_as!(
            GitRepositoryGenerationRow,
            "src/sql/db/git_repository_generations/get_git_repository_generation.sql",
            group_id,
            repository_key,
            generation_key
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitRepositoryGeneration::from_row).transpose()
    }

    /// Lists the generations of one repository, newest first.
    pub async fn list_git_repository_generations(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Vec<StoredGitRepositoryGeneration>> {
        let rows = sqlx::query_file_as!(
            GitRepositoryGenerationRow,
            "src/sql/db/git_repository_generations/list_git_repository_generations.sql",
            group_id,
            repository_key
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(StoredGitRepositoryGeneration::from_row)
            .collect()
    }

    /// Reads the generation a repository owned by `group_id` currently serves
    /// from, if any.
    pub async fn get_git_active_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Option<StoredGitActiveGeneration>> {
        let row = sqlx::query_file_as!(
            GitActiveGenerationRow,
            "src/sql/db/git_repository_generations/get_git_active_generation.sql",
            group_id,
            repository_key
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(StoredGitActiveGeneration::from_row).transpose()
    }

    /// Records the exact code destination and acknowledged point count of one
    /// building generation of a repository owned by `group_id`.
    ///
    /// Returns an error, writing nothing, when the generation is unknown,
    /// foreign, or no longer building. The statement joins the canonical source
    /// row, so one group can never write metadata onto another group's
    /// generation.
    pub(crate) async fn record_git_generation_code_vectors(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        code: &NewGitGenerationCodeVectors,
    ) -> Result<StoredGitGenerationCodeVectors> {
        let row = sqlx::query_file_as!(
            GitGenerationCodeVectorsRow,
            "src/sql/db/git_repository_generations/record_git_generation_code_vectors.sql",
            group_id,
            repository_key,
            generation_key,
            code.fingerprint,
            code.endpoint_identity,
            code.dimensions,
            code.collection_name,
            code.point_count
        )
        .fetch_optional(&self.pool)
        .await?
        .context(
            "no building git index generation of this group and repository to record code vectors",
        )?;
        let recorded = StoredGitGenerationCodeVectors::from_row(row);
        self.get_git_generation_code_vectors(group_id, repository_key, generation_key)
            .await?
            .filter(|stored| stored.fingerprint == recorded.fingerprint)
            .context("recorded git generation code vectors are not readable back")
    }

    /// Lists non-active generations that may be removed for `repository_key`.
    pub(crate) async fn list_git_generation_cleanup_candidates(
        &self,
        group_id: i64,
        repository_key: Uuid,
        retention_count: i64,
    ) -> Result<Vec<Uuid>> {
        let rows = sqlx::query_file!(
            "src/sql/db/git_repository_generations/list_git_generation_cleanup_candidates.sql",
            group_id,
            repository_key,
            retention_count
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.generation_key)
            .collect())
    }

    /// Lists sources with a generation whose vector cleanup is still pending.
    ///
    /// The source's own `retention_count` is returned with it so one maintenance
    /// pass can rank and remove the outstanding generations without requiring
    /// another index run.
    pub(crate) async fn list_pending_git_generation_cleanup_sources(
        &self,
        limit: i64,
    ) -> Result<Vec<(i64, Uuid, i64)>> {
        let rows = sqlx::query_file!(
            "src/sql/db/git_repository_generations/list_pending_git_generation_cleanup_sources.sql",
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.group_id, row.repository_key, row.retention_count))
            .collect())
    }

    /// Defers a source's generation-cleanup scheduling without recording a
    /// cleanup failure.
    ///
    /// Called by the scheduler when an index still holds the source's lifetime
    /// guard, so one long-running source cannot consume every bounded
    /// maintenance batch. Returns whether a live source was deferred.
    pub(crate) async fn defer_git_generation_cleanup_scheduling(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repository_generations/defer_git_generation_cleanup_scheduling.sql",
            group_id,
            repository_key
        )
        .execute(&self.pool)
        .await?;
        Ok(row.rows_affected() > 0)
    }

    /// Reads every generation of a repository with its optional code destination.
    pub(crate) async fn list_git_generations_with_code_vectors(
        &self,
        group_id: i64,
        repository_key: Uuid,
    ) -> Result<Vec<GitGenerationCleanupTarget>> {
        let rows = sqlx::query_file_as!(
            GitGenerationCleanupRow,
            "src/sql/db/git_repository_generations/list_git_generations_with_code_vectors.sql",
            group_id,
            repository_key
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(GitGenerationCleanupRow::into_target)
            .collect())
    }

    /// Records a bounded vector-cleanup failure for one generation.
    pub(crate) async fn mark_git_generation_cleanup_pending(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        error_code: &str,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repository_generations/mark_git_generation_cleanup_pending.sql",
            group_id,
            repository_key,
            generation_key,
            error_code
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Deletes one non-active generation after its vectors are gone.
    pub(crate) async fn delete_git_repository_generation(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/db/git_repository_generations/delete_git_repository_generation.sql",
            group_id,
            repository_key,
            generation_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Reads the code vector metadata of one generation of a repository owned by
    /// `group_id`, or `None` when that generation has no recorded code vectors.
    pub(crate) async fn get_git_generation_code_vectors(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
    ) -> Result<Option<StoredGitGenerationCodeVectors>> {
        let row = sqlx::query_file_as!(
            GitGenerationCodeVectorsRow,
            "src/sql/db/git_repository_generations/get_git_generation_code_vectors.sql",
            group_id,
            repository_key,
            generation_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(StoredGitGenerationCodeVectors::from_row))
    }
}
