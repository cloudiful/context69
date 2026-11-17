//! Creation of one file-level reprocessing task (issue 83 P1).
//!
//! A reprocessing admission is a fresh-source fork of the named library files:
//! it reuses the `rerun_source` origin so the worker runs the durable
//! replacement engine, but it starts from explicit file ids instead of the
//! items of one source task. The caller owns the transaction and has already
//! taken the sorted per-file advisory locks, so the parent task and its items
//! are created while the files are still claimed.

use anyhow::Result;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::INITIAL_ITEM_STAGE;
use super::execution::FIRST_GENERATION;
use crate::db::Database;

/// One file admitted into the new reprocessing task.
#[derive(Debug, Clone)]
pub(crate) struct NewFileReprocessItem {
    pub file_id: Uuid,
    pub payload: serde_json::Value,
}

/// The created parent task and its item ids, in admission order.
#[derive(Debug, Clone)]
pub(crate) struct CreatedFileReprocessTask {
    pub task_id: Uuid,
    pub item_ids: Vec<Uuid>,
}

impl Database {
    /// Create the `file_batch` / `rerun_source` parent for the accepted files.
    ///
    /// The transaction belongs to the caller so the per-file locks taken
    /// during admission stay held until the items exist.
    pub(crate) async fn create_file_reprocess_task_in(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user_id: i64,
        group_id: i64,
        group_path: &str,
        items: &[NewFileReprocessItem],
    ) -> Result<CreatedFileReprocessTask> {
        let task_id = Uuid::new_v4();
        let total = i64::try_from(items.len())?;
        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            task_id,
            user_id,
            group_id,
            "file_batch",
            group_path,
            Option::<String>::None,
            "rerun_source",
            total
        )
        .fetch_one(&mut **transaction)
        .await?;

        let queue = self.execution_queue().await?;
        let mut item_ids = Vec::with_capacity(items.len());
        for (ordinal, item) in items.iter().enumerate() {
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            let segment = super::execution::produce_segment(
                queue,
                transaction,
                super::execution::SegmentPayload::new(item_id, task_id, FIRST_GENERATION),
            )
            .await?;
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                task_id,
                ordinal as i32,
                item.payload,
                INITIAL_ITEM_STAGE,
                Some(item.file_id),
                Option::<Uuid>::None,
                segment.as_uuid(),
                FIRST_GENERATION
            )
            .execute(&mut **transaction)
            .await?;
        }

        Ok(CreatedFileReprocessTask { task_id, item_ids })
    }
}
