-- Requeues the exact parked item of a paused vector rebuild (issue 1212 P3).
--
-- Only the item that is waiting behind the paused dependency and owns the
-- running barrier is reopened: the join against the epoch row is the ownership
-- check, so a superseded or newer item is never requeued by a stale resume. The
-- row lock is taken on the item before the caller locks the epoch, keeping the
-- canonical item-then-epoch order.
--
-- The cursor and the barrier stay untouched here: the resume keeps the committed
-- progress and only restarts the collapsed item at `processing`, exactly like
-- the existing manual retry. The explicit resume resets the attempt count, so the
-- paused wait itself never spends an attempt.
WITH locked AS (
    SELECT item.id, item.execution_task_id
    FROM context69.task_items item
    JOIN context69.search_registry_epoch epoch ON epoch.id = 1
    WHERE item.task_id = $1
      AND item.status = 'waiting'
      AND item.waiting_reason = 'dependency'
      AND item.dependency_key = 'vector_rebuild_paused'
      AND epoch.rebuild_state = 'running'
      AND epoch.rebuild_pause_requested
      AND epoch.rebuild_owner_item = item.id
    FOR UPDATE OF item
), resumed AS (
    UPDATE context69.task_items item
    SET status = 'queued',
        stage = 'processing',
        attempt_count = 0,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = now(),
        failure_stage = NULL,
        error_message = NULL,
        retryable = TRUE,
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = NULL,
        updated_at = now()
    FROM locked
    WHERE item.id = locked.id
    RETURNING item.id, item.execution_task_id
)
SELECT id, execution_task_id FROM resumed
