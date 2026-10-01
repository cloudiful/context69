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
use uuid::Uuid;

use super::generation_types::{
    GitGenerationCoverage, NewGitRepositoryGeneration, StoredGitActiveGeneration,
    StoredGitRepositoryGeneration,
};
use super::rows::{GitActiveGenerationRow, GitRepositoryGenerationRow};
use crate::db::Database;

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
}
