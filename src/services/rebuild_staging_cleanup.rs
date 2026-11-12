//! Bounded drain of rebuild staging for completed or cancelled targets.
//!
//! Completion and cancellation enqueue only their own target in the same
//! transaction that changes the barrier. This module drains at most one queued
//! target on the existing maintenance tick: the registry epoch lock serializes
//! it against every rebuild writer, so it can never delete the staging of the
//! running or failed barrier that may still resume. Remaining rows are the
//! progress; the address row is removed only once every row family of the
//! target is empty. No timer, lease, retry counter or job kind is added.

use anyhow::Result;

use crate::db::Database;

/// Rows deleted per staging row family in one tick.
const DELETE_BATCH: i64 = 2000;

/// Drain at most one queued staging target on the caller's existing tick.
pub async fn run_once(db: &Database) -> Result<()> {
    let mut transaction = db.pool().begin().await?;
    db.lock_search_registry_epoch_in(&mut transaction).await?;
    let Some(target) = db
        .read_rebuild_staging_cleanup_target_in(&mut transaction)
        .await?
    else {
        transaction.rollback().await.ok();
        return Ok(());
    };
    let originals = db
        .delete_rebuild_staging_original_batch_in(&mut transaction, &target, DELETE_BATCH)
        .await?;
    let translations = db
        .delete_rebuild_staging_translation_batch_in(&mut transaction, &target, DELETE_BATCH)
        .await?;
    let documents = if originals == 0 && translations == 0 {
        db.delete_rebuild_staging_documents_batch_in(&mut transaction, &target, DELETE_BATCH)
            .await?
    } else {
        0
    };
    let removed = db
        .remove_rebuild_staging_cleanup_target_in(&mut transaction, &target)
        .await?;
    transaction.commit().await?;
    if removed {
        tracing::info!(target = %target, "rebuild staging cleanup drained a target");
    } else if originals + translations + documents > 0 {
        tracing::info!(
            target = %target,
            originals,
            translations,
            documents,
            "rebuild staging cleanup deleted a bounded batch"
        );
    }
    Ok(())
}
