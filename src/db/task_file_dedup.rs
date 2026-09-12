//! Per-file deduplication and authorization for library file processing.
//!
//! File-ingest tasks (`file_batch` / `url_batch` / `text_batch`) must agree on
//! a single active owner per `library_files` row. Every path that can hand a
//! file to a task — new submission, failed-item retry, terminal-task rerun —
//! validates that the file belongs to the task's group, takes the same sorted
//! per-file advisory lock, and only then looks for another active task.
//! Keeping the rules in one module stops the lock and authorization semantics
//! from drifting between paths.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredActiveFileItem {
    task_id: Uuid,
    file_id: Option<Uuid>,
}

/// Which submitted payloads may create new items after deduplication.
#[derive(Debug, Clone)]
pub(crate) struct FileDedupResolution {
    pub retained_indices: Vec<usize>,
    /// Distinct active tasks already covering the requested files, in first
    /// requested order.
    pub active_task_ids: Vec<Uuid>,
}

/// How a submission should proceed after deduplication.
#[derive(Debug, Clone)]
pub(crate) enum FileDedupDecision {
    /// Create items for these payload indices.
    Retain(Vec<usize>),
    /// Every payload is already covered by this single active task.
    Reuse(Uuid),
}

impl FileDedupResolution {
    /// Turn a resolution into a create-or-reuse decision.
    ///
    /// A partially overlapping batch is rejected atomically so the caller
    /// never receives a `TaskRef` with silently missing files.
    pub(crate) fn decide(self, payload_count: usize) -> Result<FileDedupDecision> {
        let dropped = payload_count.saturating_sub(self.retained_indices.len());
        if dropped > 0 && !self.retained_indices.is_empty() {
            anyhow::bail!(
                "file processing conflict: {dropped} requested file(s) already have an active processing task"
            );
        }
        if self.retained_indices.is_empty() && payload_count > 0 {
            if self.active_task_ids.len() != 1 {
                anyhow::bail!(
                    "file processing conflict: requested files are already covered by multiple active processing tasks"
                );
            }
            return Ok(FileDedupDecision::Reuse(self.active_task_ids[0]));
        }
        Ok(FileDedupDecision::Retain(self.retained_indices))
    }
}

/// `file_id` carried by a task payload, when present and well-formed.
pub(crate) fn declared_file_id(payload: &Value) -> Option<Uuid> {
    payload
        .get("file_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
}

/// Reject the batch when any requested file does not exist in `group_id`.
///
/// This runs before any advisory lock or active-task lookup. The error is
/// deliberately indistinguishable between "missing" and "another group's
/// file" so a caller cannot probe foreign file ids.
pub(crate) async fn validate_file_ownership(
    connection: &mut PgConnection,
    group_id: Option<i64>,
    file_ids: &[Uuid],
) -> Result<()> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let group_id = group_id.context("file processing requires an owning group")?;
    let mut requested = file_ids.to_vec();
    requested.sort_unstable();
    requested.dedup();
    let owned = sqlx::query_file_scalar!(
        "src/sql/db/tasks/owned_file_count.sql",
        group_id,
        &requested
    )
    .fetch_one(&mut *connection)
    .await?;
    if owned != requested.len() as i64 {
        anyhow::bail!("library file not found in the task group");
    }
    Ok(())
}

/// Reject the batch when any requested file had its source deliberately
/// released. A released file cannot be reprocessed; re-uploading its bytes is
/// a new upload, not a reprocess. Runs after ownership so a foreign file is
/// still indistinguishable from a missing one.
pub(crate) async fn validate_files_not_released(
    connection: &mut PgConnection,
    group_id: Option<i64>,
    file_ids: &[Uuid],
) -> Result<()> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let group_id = group_id.context("file processing requires an owning group")?;
    let mut requested = file_ids.to_vec();
    requested.sort_unstable();
    requested.dedup();
    let released = sqlx::query_file_scalar!(
        "src/sql/db/tasks/released_file_count.sql",
        group_id,
        &requested
    )
    .fetch_one(&mut *connection)
    .await?;
    if released > 0 {
        anyhow::bail!("file processing conflict: released file source cannot be reprocessed");
    }
    Ok(())
}

/// Lock the per-file processing slot for every requested file.
///
/// Locks are transaction-scoped advisory locks taken in sorted order so
/// create/retry/rerun serialize per file and multi-file batches cannot
/// deadlock. Callers must validate group ownership first: the key is only the
/// file id.
pub(crate) async fn lock_file_processing_slots(
    connection: &mut PgConnection,
    file_ids: &[Uuid],
) -> Result<()> {
    let mut sorted = file_ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for file_id in sorted {
        sqlx::query_file!(
            "src/sql/db/tasks/lock_task_file.sql",
            format!("library_file_processing:{file_id}")
        )
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

/// Validate ownership and lock every file a retry or rerun may touch. The
/// released check runs after the lock so it cannot race a concurrent release.
pub(crate) async fn claim_file_processing_slots(
    connection: &mut PgConnection,
    group_id: Option<i64>,
    file_ids: &[Uuid],
) -> Result<()> {
    if file_ids.is_empty() {
        return Ok(());
    }
    validate_file_ownership(connection, group_id, file_ids).await?;
    lock_file_processing_slots(connection, file_ids).await?;
    validate_files_not_released(connection, group_id, file_ids).await
}

/// Claim the processing slots of every file referenced by a task's failed
/// items, before the retry's active-sibling check runs.
pub(crate) async fn claim_failed_item_file_slots(
    connection: &mut PgConnection,
    group_id: Option<i64>,
    task_id: Uuid,
) -> Result<()> {
    let file_ids = sqlx::query_file_scalar!("src/sql/db/tasks/failed_item_file_ids.sql", task_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<Uuid>>();
    claim_file_processing_slots(connection, group_id, &file_ids).await
}

/// Claim the processing slots of every file referenced by a task's
/// non-succeeded items, before the rerun's active-sibling check runs.
pub(crate) async fn claim_unfinished_item_file_slots(
    connection: &mut PgConnection,
    group_id: Option<i64>,
    task_id: Uuid,
) -> Result<()> {
    let file_ids =
        sqlx::query_file_scalar!("src/sql/db/tasks/unfinished_item_file_ids.sql", task_id)
            .fetch_all(&mut *connection)
            .await?
            .into_iter()
            .flatten()
            .collect::<Vec<Uuid>>();
    claim_file_processing_slots(connection, group_id, &file_ids).await
}

/// Resolve which explicitly named files in a submission may create new items.
///
/// Files already owned by an active task in the same group are removed from
/// `retained_indices`; `active_task_ids` reports the covering tasks so the
/// caller can reuse or reject the batch.
pub(crate) async fn resolve_file_dedup(
    connection: &mut PgConnection,
    group_id: Option<i64>,
    payloads: &[Value],
) -> Result<FileDedupResolution> {
    let all_indices = (0..payloads.len()).collect::<Vec<_>>();
    let mut explicit = payloads
        .iter()
        .filter_map(declared_file_id)
        .collect::<Vec<_>>();
    if explicit.is_empty() {
        return Ok(FileDedupResolution {
            retained_indices: all_indices,
            active_task_ids: Vec::new(),
        });
    }
    explicit.sort_unstable();
    explicit.dedup();
    validate_file_ownership(connection, group_id, &explicit).await?;
    lock_file_processing_slots(connection, &explicit).await?;
    validate_files_not_released(connection, group_id, &explicit).await?;
    let owner_group_id = group_id.context("file processing requires an owning group")?;
    let active = sqlx::query_file_as!(
        StoredActiveFileItem,
        "src/sql/db/tasks/find_active_file_items.sql",
        &explicit,
        owner_group_id
    )
    .fetch_all(&mut *connection)
    .await?;
    let blocked = active
        .iter()
        .filter_map(|row| row.file_id.map(|file_id| (file_id, row.task_id)))
        .collect::<HashMap<_, _>>();
    let mut retained_indices = all_indices;
    retained_indices.retain(|&index| {
        declared_file_id(&payloads[index]).is_none_or(|file_id| !blocked.contains_key(&file_id))
    });
    let mut active_task_ids = Vec::new();
    for file_id in &explicit {
        if let Some(task_id) = blocked.get(file_id)
            && !active_task_ids.contains(task_id)
        {
            active_task_ids.push(*task_id);
        }
    }
    Ok(FileDedupResolution {
        retained_indices,
        active_task_ids,
    })
}
