//! Group-scoped manifest persistence for generation code content (issue #681
//! work unit 3B2).
//!
//! The manifest of a snapshot is written whole: one statement stores the
//! deduplicated bytes, retires the entries the new manifest no longer
//! describes, and inserts the new ones. Every call is scoped to a building
//! generation of a repository owned by the calling group, and the group's
//! current visibility is read from `context69.groups` on every query rather
//! than from a stored copy.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use uuid::Uuid;

use super::file_rows::{GitGenerationFileRow, GitManifestReplaceRow};
use super::file_types::{
    GitGenerationManifest, GitManifestReplacement, MAX_GIT_FILE_BYTES, MAX_GIT_GENERATION_BYTES,
    MAX_GIT_LIST_PAGE, MAX_GIT_MANIFEST_FILES, NewGitGenerationFile, StoredGitGenerationFile,
};
use crate::db::Database;
use crate::domain_errors::DomainError;

impl Database {
    /// Replaces the path manifest of one building generation, together with the
    /// raw bytes it needs, in a single statement.
    ///
    /// Bytes are stored once per (generation, provider blob id), so a manifest
    /// describing the same content at several paths pays for it once. Manifest
    /// entries the call omits are retired with their chunks, and bytes no
    /// described entry needs any more go with them; entries the call repeats
    /// unchanged keep their identity and their chunks.
    ///
    /// Returns an error, writing nothing, when the generation is unknown, is not
    /// of this group, or is no longer building, and when the manifest maps one
    /// blob id to different bytes or describes one path twice.
    pub async fn replace_git_generation_files(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        files: &[NewGitGenerationFile],
    ) -> Result<GitManifestReplacement> {
        let manifest = validate_manifest(files)?;
        let row = sqlx::query_file_as!(
            GitManifestReplaceRow,
            "src/sql/db/git_repository_files/replace_git_generation_files.sql",
            group_id,
            repository_key,
            generation_key,
            &manifest.paths,
            &manifest.languages,
            &manifest.provider_blob_shas,
            &manifest.contents,
            &manifest.line_counts
        )
        .fetch_one(&self.pool)
        .await?;
        GitManifestReplacement::from_row(row)
    }

    /// Reads one manifest entry of a generation owned by `group_id`.
    pub async fn get_git_generation_file(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        path: &str,
    ) -> Result<Option<StoredGitGenerationFile>> {
        let row = sqlx::query_file_as!(
            GitGenerationFileRow,
            "src/sql/db/git_repository_files/get_git_generation_file.sql",
            group_id,
            repository_key,
            generation_key,
            path
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(StoredGitGenerationFile::from_row))
    }

    /// Lists one bounded page of a generation's manifest, ordered by path.
    pub async fn list_git_generation_files(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredGitGenerationFile>> {
        bounded_page(limit, offset)?;
        let rows = sqlx::query_file_as!(
            GitGenerationFileRow,
            "src/sql/db/git_repository_files/list_git_generation_files.sql",
            group_id,
            repository_key,
            generation_key,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(StoredGitGenerationFile::from_row)
            .collect())
    }
}

/// Reject a manifest the storage layer could not accept anyway, so the caller
/// gets a domain error instead of a constraint violation inside its
/// transaction. The aggregate is measured over deduplicated blob ids, the same
/// bytes the database counts.
pub(super) fn validate_manifest(files: &[NewGitGenerationFile]) -> Result<GitGenerationManifest> {
    if files.len() > MAX_GIT_MANIFEST_FILES {
        return Err(DomainError::payload_too_large("git_manifest_too_large").into());
    }
    let mut stored_bytes_by_blob: BTreeMap<&str, i64> = BTreeMap::new();
    for file in files {
        let byte_count =
            i64::try_from(file.content.len()).context("git file content length is out of range")?;
        if byte_count > MAX_GIT_FILE_BYTES as i64 {
            return Err(DomainError::payload_too_large("git_file_too_large").into());
        }
        if file.line_count < 0 {
            return Err(DomainError::invalid_argument("git_line_count_invalid").into());
        }
        // The database refuses content that is not storable text; checking it
        // here as well turns a rejected file into a domain error the caller can
        // exclude by path, instead of a constraint violation that would abort
        // the caller's transaction. `CodeText::validate` in the acquisition
        // service is the producer-side gate for the same rule.
        match std::str::from_utf8(&file.content) {
            Ok(text) if text.contains('\0') => {
                return Err(DomainError::invalid_argument("git_code_text_contains_nul").into());
            }
            Ok(_) => {}
            Err(_) => {
                return Err(DomainError::invalid_argument("git_code_text_not_utf8").into());
            }
        }
        stored_bytes_by_blob
            .entry(file.provider_blob_sha.as_str())
            .or_insert(byte_count);
    }
    let stored_bytes: i64 = stored_bytes_by_blob.values().sum();
    if stored_bytes > MAX_GIT_GENERATION_BYTES {
        return Err(DomainError::payload_too_large("git_generation_budget_exceeded").into());
    }
    Ok(GitGenerationManifest::build(files))
}

/// One bounded page: a caller cannot ask for an unbounded listing.
pub(super) fn bounded_page(limit: i64, offset: i64) -> Result<()> {
    if !(1..=MAX_GIT_LIST_PAGE).contains(&limit) || offset < 0 {
        return Err(DomainError::invalid_argument("git_page_out_of_bounds").into());
    }
    Ok(())
}
