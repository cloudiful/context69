//! Retention cleanup, source deletion, and remote vector ownership.

use std::future::Future;

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::QdrantConfig;
use crate::db::task_git_dedup::{active_git_index_task, lock_git_index_admission};
use crate::db::{Database, GitIndexLifetimeGuard, StoredGitGenerationCodeVectors};
use crate::domain_errors::DomainError;
use crate::qdrant_index::QdrantIndex;
use crate::services::app::Context69App;
use crate::services::git_repository::code_vectors::code_endpoint_identity;

use super::GIT_REFRESH_BATCH;

/// Largest cleanup error code retained on a generation.
const MAX_CLEANUP_ERROR_CHARS: usize = 256;

/// Retries deletions whose earlier vector cleanup failed.
///
/// The batch is selected by due retry time, so a permanent failure is backed
/// off instead of occupying every bounded pass.
pub(crate) async fn retry_pending_deletions(app: &Context69App) -> Result<()> {
    let pending = app.db.list_pending_git_deletions(GIT_REFRESH_BATCH).await?;
    for (group_id, repository_key) in pending {
        match delete_repository(app, group_id, repository_key).await {
            Ok(true) => {
                tracing::info!(%repository_key, "retried git repository deletion completed");
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    %repository_key,
                    %error,
                    "pending git repository deletion still cannot clean its vectors"
                );
            }
        }
    }
    Ok(())
}

/// Retries vector cleanup for generations left pending without a new index.
///
/// Every source with a pending generation is considered, including manual-only
/// and disabled sources. The pass is bounded per tick and reports per-source
/// failures without stopping the remaining sources.
pub(crate) async fn retry_pending_generation_cleanup(app: &Context69App) -> Result<()> {
    let pending = app
        .db
        .list_pending_git_generation_cleanup_sources(GIT_REFRESH_BATCH)
        .await?;
    if pending.is_empty() {
        return Ok(());
    }
    let qdrant_url = app.settings.runtime_qdrant_url().await?;
    run_pending_generation_cleanup(
        &app.db,
        pending,
        |group_id, repository_key, generation_key, metadata| {
            let qdrant_url = qdrant_url.clone();
            async move {
                remove_generation_vectors(
                    group_id,
                    repository_key,
                    generation_key,
                    metadata.as_ref(),
                    qdrant_url.as_deref(),
                )
                .await
            }
        },
    )
    .await
}

/// Runs one bounded generation-cleanup pass over the supplied sources.
///
/// The cleanup callback owns the remote deletion so the independent pass can be
/// exercised without another index run.
pub(crate) async fn run_pending_generation_cleanup<F, Fut>(
    db: &Database,
    pending: Vec<(i64, Uuid, i64)>,
    mut remove: F,
) -> Result<()>
where
    F: FnMut(i64, Uuid, Uuid, Option<StoredGitGenerationCodeVectors>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    for (group_id, repository_key, retention_count) in pending {
        let mut cleanup =
            |generation_key, metadata| remove(group_id, repository_key, generation_key, metadata);
        match prune_generations_with_deferral(
            db,
            group_id,
            repository_key,
            retention_count,
            true,
            &mut cleanup,
        )
        .await
        {
            Ok(failures) => {
                tracing::info!(
                    %repository_key,
                    failures,
                    "retried pending git generation cleanup"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %repository_key,
                    %error,
                    "pending git generation cleanup still cannot be retried"
                );
            }
        }
    }
    Ok(())
}

/// Completes one successful run: consumes only the observed demand and prunes
/// retired generations.
pub(crate) async fn complete_refresh(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    observed_request: Option<DateTime<Utc>>,
    retention_count: i64,
    qdrant_url: Option<&str>,
) -> Result<()> {
    if let Some(observed) = observed_request {
        db.consume_git_refresh_request(group_id, repository_key, observed)
            .await?;
    }
    let failures =
        prune_generations(db, group_id, repository_key, retention_count, qdrant_url).await?;
    if failures > 0 {
        tracing::warn!(
            %repository_key,
            failures,
            "git generation vector cleanup is pending a retry"
        );
    }
    Ok(())
}

/// Removes generations beyond retention, returning how many vector cleanups
/// still need a retry. Cleanup failures are recorded, never silently ignored.
pub(crate) async fn prune_generations(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    retention_count: i64,
    qdrant_url: Option<&str>,
) -> Result<usize> {
    let mut remove = |generation_key, metadata: Option<StoredGitGenerationCodeVectors>| async move {
        remove_generation_vectors(
            group_id,
            repository_key,
            generation_key,
            metadata.as_ref(),
            qdrant_url,
        )
        .await
    };
    prune_generations_with_deferral(
        db,
        group_id,
        repository_key,
        retention_count,
        false,
        &mut remove,
    )
    .await
}

/// Optionally deferring the source's scheduling when an index still holds the
/// lifetime guard.
///
/// Only the independent maintenance pass sets `defer_when_held`; an index
/// completion that runs under its own guard must never record a scheduler
/// deferral.
async fn prune_generations_with_deferral<F, Fut>(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    retention_count: i64,
    defer_when_held: bool,
    remove: &mut F,
) -> Result<usize>
where
    F: FnMut(Uuid, Option<StoredGitGenerationCodeVectors>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // Cleanup defers instead of racing an index that is still building or
    // writing vectors. The guard is held through the remote deletion and the
    // metadata deletion, so an index that starts afterwards cannot observe a
    // half-cleaned generation.
    let Some(guard) = GitIndexLifetimeGuard::try_acquire(db.pool(), repository_key).await? else {
        tracing::info!(
            %repository_key,
            "git generation cleanup deferred while an index owns the repository"
        );
        if defer_when_held
            && let Err(error) = db
                .defer_git_generation_cleanup_scheduling(group_id, repository_key)
                .await
        {
            tracing::warn!(
                %repository_key,
                %error,
                "failed to defer git generation cleanup scheduling"
            );
        }
        return Ok(0);
    };
    let result =
        prune_generations_locked(db, group_id, repository_key, retention_count, remove).await;
    if let Err(error) = guard.release().await {
        tracing::warn!(%repository_key, %error, "git generation cleanup lock release failed");
    }
    result
}

async fn prune_generations_locked<F, Fut>(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    retention_count: i64,
    remove: &mut F,
) -> Result<usize>
where
    F: FnMut(Uuid, Option<StoredGitGenerationCodeVectors>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let candidates = db
        .list_git_generation_cleanup_candidates(group_id, repository_key, retention_count)
        .await?;
    let mut failures = 0usize;
    for generation_key in candidates {
        let metadata = db
            .get_git_generation_code_vectors(group_id, repository_key, generation_key)
            .await?;
        match remove(generation_key, metadata).await {
            Ok(()) => {
                if !db
                    .delete_git_repository_generation(group_id, repository_key, generation_key)
                    .await?
                {
                    tracing::warn!(
                        %generation_key,
                        "generation cleanup skipped because it is active or gone"
                    );
                }
            }
            Err(error) => {
                failures += 1;
                let code = bounded_cleanup_error(&error);
                if let Err(mark_error) = db
                    .mark_git_generation_cleanup_pending(
                        group_id,
                        repository_key,
                        generation_key,
                        &code,
                    )
                    .await
                {
                    tracing::warn!(
                        %generation_key,
                        %mark_error,
                        "failed to record pending git generation cleanup"
                    );
                }
                tracing::warn!(
                    %generation_key,
                    %error,
                    "git generation vector cleanup failed; metadata retained"
                );
            }
        }
    }
    Ok(failures)
}

/// Deletes a source after every recorded generation's vectors are removed.
///
/// The admission lock and the active-task check run in one transaction, so a
/// delete that commits also prevents a later submission from admitting work.
/// Returns `Ok(false)` when the source no longer exists.
pub(crate) async fn delete_repository(
    app: &Context69App,
    group_id: i64,
    repository_key: Uuid,
) -> Result<bool> {
    let mut tx = app.db.pool().begin().await?;
    lock_git_index_admission(&mut tx, repository_key).await?;
    if active_git_index_task(&mut tx, repository_key)
        .await?
        .is_some()
    {
        tx.rollback().await?;
        bail!(DomainError::conflict("git_repository_busy"));
    }
    if !app
        .db
        .mark_git_repository_deletion_requested_in(&mut tx, group_id, repository_key)
        .await?
    {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.commit().await?;

    let qdrant_url = app.settings.runtime_qdrant_url().await?;
    match cleanup_deleted_source(
        &app.db,
        group_id,
        repository_key,
        |generation_key, metadata| {
            let qdrant_url = qdrant_url.clone();
            async move {
                remove_generation_vectors(
                    group_id,
                    repository_key,
                    generation_key,
                    metadata.as_ref(),
                    qdrant_url.as_deref(),
                )
                .await
            }
        },
    )
    .await
    {
        Ok(Some(deleted)) => Ok(deleted),
        Ok(None) => bail!(DomainError::conflict("git_repository_busy")),
        Err(error) => Err(error),
    }
}

/// Removes a pending source's vectors, then its ownership row.
///
/// The lifetime guard is tried, not awaited: a held guard means an index still
/// owns the repository, so cleanup defers without recording a failure. A real
/// cleanup failure records the bounded retry before the guard is released.
/// `Ok(None)` means the guard was held; `Ok(Some(false))` means the source was
/// already gone.
pub(crate) async fn cleanup_deleted_source<F, Fut>(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    mut remove: F,
) -> Result<Option<bool>>
where
    F: FnMut(Uuid, Option<StoredGitGenerationCodeVectors>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let Some(guard) = GitIndexLifetimeGuard::try_acquire(db.pool(), repository_key).await? else {
        tracing::info!(
            %repository_key,
            "source deletion deferred while an index owns the repository"
        );
        return Ok(None);
    };
    let result = async {
        let generations = db
            .list_git_generations_with_code_vectors(group_id, repository_key)
            .await?;
        for generation in &generations {
            remove(generation.generation_key, generation.code_vectors.clone())
                .await
                .map_err(|error| {
                    tracing::warn!(
                        generation_key = %generation.generation_key,
                        %error,
                        "source deletion is waiting for vector cleanup"
                    );
                    error
                })?;
        }
        db.delete_git_repository_source(group_id, repository_key)
            .await
    }
    .await;
    if result.is_err()
        && let Err(error) = db
            .mark_git_repository_deletion_retry(group_id, repository_key)
            .await
    {
        tracing::warn!(%repository_key, %error, "failed to record the source deletion retry");
    }
    if let Err(error) = guard.release().await {
        tracing::warn!(%repository_key, %error, "source deletion lock release failed");
    }
    result.map(Some)
}

/// Removes the remote points of one generation when it has a code destination.
///
/// A lexical generation needs no remote work. Absent metadata means no remote
/// write was attempted, because the destination is recorded before the first
/// upsert; it is never evidence that an unrecorded write can be skipped.
///
/// The recorded endpoint identity must match the currently configured Qdrant
/// endpoint before any remote read. A different endpoint means the points on
/// the old destination are not reachable from here, and an unknown identity is a
/// row recorded before that identity existed. Both fail closed and retain the
/// metadata, so the only record of those points is never dropped. The
/// collection is attached strictly; a shape mismatch or an unreachable
/// destination also fails so the metadata is retained for a retry.
pub(crate) async fn remove_generation_vectors(
    group_id: i64,
    repository_key: Uuid,
    generation_key: Uuid,
    metadata: Option<&StoredGitGenerationCodeVectors>,
    qdrant_url: Option<&str>,
) -> Result<()> {
    let Some(metadata) = metadata else {
        return Ok(());
    };
    let url = qdrant_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| anyhow!(DomainError::conflict("git_code_qdrant_url_missing")))?;
    let stored_identity = metadata
        .endpoint_identity
        .as_deref()
        .ok_or_else(|| anyhow!(DomainError::conflict("git_code_endpoint_identity_unknown")))?;
    if stored_identity != code_endpoint_identity(url) {
        return Err(anyhow!(DomainError::conflict(
            "git_code_endpoint_identity_mismatch"
        )));
    }
    let dimensions = usize::try_from(metadata.dimensions)
        .map_err(|_| anyhow!(DomainError::internal("git_code_dimensions_out_of_range")))?;
    let config = QdrantConfig {
        url: url.to_string(),
        collection_name: metadata.collection_name.clone(),
        recreate_on_dimension_mismatch: false,
    };
    if QdrantIndex::inspect_collection(&config, &metadata.collection_name, dimensions)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let index = QdrantIndex::attach_code_read_only_collection(
        &config,
        &metadata.collection_name,
        dimensions,
    )
    .await?;
    index
        .delete_points_for_code_generation(group_id, repository_key, generation_key)
        .await?;
    Ok(())
}

fn bounded_cleanup_error(error: &anyhow::Error) -> String {
    let message = error.to_string();
    if message.chars().count() <= MAX_CLEANUP_ERROR_CHARS {
        return message;
    }
    message.chars().take(MAX_CLEANUP_ERROR_CHARS).collect()
}
