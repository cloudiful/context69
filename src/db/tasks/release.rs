use anyhow::Result;
use uuid::Uuid;

use super::{
    execution::{ExecutionContext, worker_id},
    item_lifecycle::lock_task_item_for_worker_in,
};
use crate::db::Database;

impl Database {
    pub(crate) async fn release_shutdown_item(
        &self,
        execution: &ExecutionContext,
        task_id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        if !lock_task_item_for_worker_in(&mut tx, execution.item_id, execution)
            .await?
            .is_some_and(|item| item.owned)
        {
            tx.rollback().await?;
            return Ok(false);
        }
        let queue = self.execution_queue().await?;
        let native = match queue
            .release_in(
                &mut tx,
                std::slice::from_ref(&execution.attempt),
                worker_id(),
            )
            .await?
            .pop()
            .expect("one attempt outcome")
        {
            Ok(native) => native,
            Err(
                cloudiful_tasks::Error::LeaseLost { .. }
                | cloudiful_tasks::Error::InvalidTaskState { .. },
            ) => {
                tx.rollback().await?;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/release_shutdown_item.sql",
            execution.item_id,
            task_id,
            execution.task_id(),
            execution.generation,
            execution.fencing_token(),
            execution.attempt_id(),
            native.status().as_str(),
            native.pause_state().as_str(),
        )
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}
