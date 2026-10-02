//! Bounded initial snapshot indexing (issue #681 work unit 3B3).
//!
//! One call indexes a single bounded public snapshot into a new group-owned
//! generation: it resolves and pins the ref, marks the source indexing, opens a
//! building generation, walks the bounded tree under explicit eligible-file and
//! Blob-fetch counts, fetches and classifies each eligible blob, writes the
//! complete manifest and its chunks, re-reads the source to reject a changed
//! target as stale, and only then activates the generation and advances the
//! indexed checkpoint.
//!
//! Nothing is activated before every included file succeeds. On any failure the
//! partial content is retired and the generation is marked failed (best effort),
//! so the previous active generation keeps serving. An empty eligible snapshot
//! completes safely. Acquisition stays provider-neutral behind an in-crate
//! factory that builds a fresh acquirer per generation, so aggregate byte
//! budgets are never reused, and public acquisition carries no credential.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::sources::GitIndexStatus;
use crate::db::{
    Database, GitCheckpointUpdate, GitGenerationCoverage, NewGitGenerationChunk,
    NewGitGenerationFile, NewGitRepositoryGeneration,
};
use crate::domain_errors::DomainError;

use super::classify::language_for_path;
use super::code_text::{CodeText, chunk_code_text};
use super::github_client::{GitHubAcquirer, RepositoryAcquirer};
use super::github_transport::GitHttpTransport;
use super::limits::{GitIndexLimits, plan_snapshot_files};
use super::model::GitAcquisitionLimits;
use super::ref_path_safety::{SafeRef, SafeSha};

/// Stable, bounded error code recorded on a failed generation. The concrete
/// failure stays in the returned error, where a caller can classify it.
const GENERATION_FAILURE_CODE: &str = "git_index_failed";

/// What one completed snapshot indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitSnapshotIndexOutcome {
    pub repository_key: Uuid,
    pub generation_key: Uuid,
    pub generation_number: i64,
    pub ref_name: String,
    pub commit_sha: String,
    pub file_count: i64,
    pub excluded_file_count: i64,
    pub total_bytes: i64,
    pub activated_at: DateTime<Utc>,
}

/// Builds an acquirer for one snapshot.
///
/// The factory is called once per generation, so every snapshot gets its own
/// aggregate byte budget instead of sharing the spend of a previous walk. A
/// provider implementation that authenticates is free to do so, but public
/// acquisition never requires a credential.
pub(crate) trait GitSnapshotProvider: Send + Sync {
    fn acquirer(
        &self,
        canonical_url: &str,
        limits: GitAcquisitionLimits,
    ) -> Result<Box<dyn RepositoryAcquirer>>;
}

/// Credential-free public GitHub provider built on the bounded 3A client.
pub(crate) struct GitHubSnapshotProvider<T: GitHttpTransport + ?Sized = dyn GitHttpTransport> {
    transport: Arc<T>,
}

impl<T: GitHttpTransport + ?Sized> GitHubSnapshotProvider<T> {
    pub(crate) fn new(transport: Arc<T>) -> Self {
        Self { transport }
    }
}

impl<T: GitHttpTransport + ?Sized + Sync + 'static> GitSnapshotProvider
    for GitHubSnapshotProvider<T>
{
    fn acquirer(
        &self,
        canonical_url: &str,
        limits: GitAcquisitionLimits,
    ) -> Result<Box<dyn RepositoryAcquirer>> {
        Ok(Box::new(GitHubAcquirer::new(
            canonical_url,
            Arc::clone(&self.transport),
            limits,
        )?))
    }
}

/// Indexes one bounded snapshot of `repository_key` into a new generation.
///
/// On success the new generation is active and the source checkpoint names the
/// pinned commit as indexed. On failure nothing is activated; partial content is
/// retired and the generation is marked failed, leaving any previous active
/// generation serving.
pub(crate) async fn index_git_repository_snapshot(
    db: &Database,
    provider: &dyn GitSnapshotProvider,
    limits: GitIndexLimits,
    group_id: i64,
    repository_key: Uuid,
) -> Result<GitSnapshotIndexOutcome> {
    let mut progress = SnapshotProgress::default();
    match attempt_snapshot(
        db,
        provider,
        &limits,
        group_id,
        repository_key,
        &mut progress,
    )
    .await
    {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            fail_snapshot(db, group_id, repository_key, progress).await;
            Err(error)
        }
    }
}

/// Tracks how far one attempt got, so a failure cleans up only what exists.
#[derive(Debug, Default)]
struct SnapshotProgress {
    /// The source was moved to `indexing`, so a failure should move it back.
    indexing_marked: bool,
    /// The building generation to retire and fail, once it exists.
    generation_key: Option<Uuid>,
    /// The target changed under the attempt; never touch the source then.
    stale: bool,
    /// The generation was activated; a later failure must not roll it back.
    activated: bool,
}

async fn attempt_snapshot(
    db: &Database,
    provider: &dyn GitSnapshotProvider,
    limits: &GitIndexLimits,
    group_id: i64,
    repository_key: Uuid,
    progress: &mut SnapshotProgress,
) -> Result<GitSnapshotIndexOutcome> {
    let source = db
        .get_git_repository_source(group_id, repository_key)
        .await?
        .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
    let ref_name = source.version.ref_name.clone();

    // A fresh acquirer owns a fresh aggregate byte budget for this generation.
    let acquirer = provider.acquirer(&source.canonical_url, limits.acquisition)?;

    // Resolve and pin the ref before any blob byte is spent. An explicit pinned
    // commit is already the pin; otherwise the ref is resolved now.
    let pinned = match source.version.commit_sha.as_deref() {
        Some(commit) => SafeSha::parse(commit)?,
        None => acquirer.resolve_ref(&SafeRef::parse(&ref_name)?).await?,
    };

    db.update_git_repository_checkpoint(
        group_id,
        repository_key,
        &GitCheckpointUpdate {
            target_commit_sha: Some(pinned.as_str().to_owned()),
            indexed_commit_sha: None,
            index_status: GitIndexStatus::Indexing,
        },
    )
    .await?
    .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
    progress.indexing_marked = true;

    let generation = db
        .start_git_repository_generation(
            group_id,
            repository_key,
            &NewGitRepositoryGeneration {
                ref_name: ref_name.clone(),
                commit_sha: pinned.as_str().to_owned(),
                index_profile: source.index_profile,
            },
        )
        .await?;
    progress.generation_key = Some(generation.generation_key);

    let listing = acquirer.list_tree(&pinned).await?;
    let mut excluded_file_count = listing.excluded_files;
    let entries = plan_snapshot_files(listing.files, limits)?;

    let mut files: Vec<NewGitGenerationFile> = Vec::with_capacity(entries.len());
    let mut chunks_by_path: Vec<(String, Vec<NewGitGenerationChunk>)> = Vec::new();
    let mut total_bytes: i64 = 0;
    let mut blob_cache: HashMap<String, Bytes> = HashMap::new();

    for entry in entries {
        let blob_key = entry.blob_sha.as_str().to_owned();
        let bytes = match blob_cache.get(&blob_key) {
            Some(cached) => cached.clone(),
            None => {
                let blob = acquirer.fetch_blob(&entry.blob_sha).await?;
                blob_cache.insert(blob_key.clone(), blob.bytes.clone());
                blob.bytes
            }
        };
        // Binary or NUL-carrying content cannot be stored as exact code text, so
        // the file is excluded and counted instead of failing the snapshot.
        let Ok(text) = CodeText::validate(bytes.as_ref()) else {
            excluded_file_count += 1;
            continue;
        };
        let Ok(chunks) = chunk_code_text(text.as_str(), limits.chunk_bounds) else {
            excluded_file_count += 1;
            continue;
        };
        let byte_count = i64::try_from(bytes.len())
            .map_err(|_| anyhow!(DomainError::payload_too_large("git_file_too_large")))?;
        total_bytes = total_bytes.saturating_add(byte_count);
        files.push(NewGitGenerationFile {
            path: entry.path.as_str().to_owned(),
            language: language_for_path(entry.path.as_str()).to_owned(),
            provider_blob_sha: blob_key,
            content: bytes.to_vec(),
            line_count: text.line_count(),
        });
        chunks_by_path.push((entry.path.as_str().to_owned(), chunk_rows(chunks)));
    }

    let coverage = GitGenerationCoverage {
        file_count: bounded_count(files.len()),
        excluded_file_count: bounded_count(excluded_file_count),
        total_bytes,
    };

    db.replace_git_generation_files(group_id, repository_key, generation.generation_key, &files)
        .await?;

    for (path, chunks) in &chunks_by_path {
        let stored = db
            .get_git_generation_file(group_id, repository_key, generation.generation_key, path)
            .await?
            .ok_or_else(|| anyhow!(DomainError::internal("git_index_manifest_entry_missing")))?;
        db.replace_git_file_chunks(
            group_id,
            repository_key,
            generation.generation_key,
            stored.file_key,
            chunks,
        )
        .await?;
    }

    // The target must still be the one this attempt pinned; a concurrent change
    // is stale and must never activate over it.
    let current = db
        .get_git_repository_source(group_id, repository_key)
        .await?
        .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;
    if current.version.ref_name != ref_name
        || current.version.commit_sha.as_deref() != Some(pinned.as_str())
    {
        progress.stale = true;
        return Err(anyhow!(DomainError::conflict("git_index_stale_target")));
    }

    let activated = db
        .complete_and_activate_git_repository_generation(
            group_id,
            repository_key,
            generation.generation_key,
            coverage,
        )
        .await?;
    progress.activated = true;

    // The checkpoint advances only after the generation is active.
    db.update_git_repository_checkpoint(
        group_id,
        repository_key,
        &GitCheckpointUpdate {
            target_commit_sha: Some(pinned.as_str().to_owned()),
            indexed_commit_sha: Some(pinned.as_str().to_owned()),
            index_status: GitIndexStatus::Ready,
        },
    )
    .await?
    .ok_or_else(|| anyhow!(DomainError::not_found("git_index_source_missing")))?;

    Ok(GitSnapshotIndexOutcome {
        repository_key,
        generation_key: generation.generation_key,
        generation_number: generation.generation_number,
        ref_name,
        commit_sha: pinned.as_str().to_owned(),
        file_count: coverage.file_count,
        excluded_file_count: coverage.excluded_file_count,
        total_bytes: coverage.total_bytes,
        activated_at: activated.activated_at,
    })
}

/// Best-effort rollback of a failed attempt: retire the partial content, mark
/// the building generation failed, and un-stick the source. A stale attempt
/// leaves the source alone because a concurrent change owns it now, and an
/// already-activated generation is never rolled back.
async fn fail_snapshot(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    progress: SnapshotProgress,
) {
    if progress.activated {
        return;
    }
    if let Some(generation_key) = progress.generation_key {
        let _ = db
            .replace_git_generation_files(group_id, repository_key, generation_key, &[])
            .await;
        let _ = db
            .fail_git_repository_generation(
                group_id,
                repository_key,
                generation_key,
                GENERATION_FAILURE_CODE,
            )
            .await;
    }
    if progress.indexing_marked && !progress.stale {
        let _ = db
            .update_git_repository_checkpoint(
                group_id,
                repository_key,
                &GitCheckpointUpdate {
                    target_commit_sha: None,
                    indexed_commit_sha: None,
                    index_status: GitIndexStatus::Failed,
                },
            )
            .await;
    }
}

fn chunk_rows(chunks: Vec<super::code_text::CodeChunk>) -> Vec<NewGitGenerationChunk> {
    chunks
        .into_iter()
        .map(|chunk| NewGitGenerationChunk {
            chunk_index: chunk.chunk_index,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            text: chunk.text,
        })
        .collect()
}

fn bounded_count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "index_generation_tests.rs"]
mod tests;
