-- The domain projection of one sealed execution segment (issue 1020 P2).
--
-- The library sealed the segment of an expired attempt. The domain attempt is
-- interrupted, the mirror of the lease is cleared, and the item is requeued
-- right away under the remaining domain attempt budget - a crashed worker is
-- not a provider failure, so it carries no backoff. The guard is the segment
-- itself: the library task and the execution generation, so a sealed segment of
-- an older generation never touches the item that replaced it.
--
-- `$1` is the item, `$2` the library task of the segment, `$3` the execution
-- generation of the segment and `$4` the domain parent task the segment was
-- produced for.
WITH target AS (
    SELECT item.id, item.task_id, item.attempt_count
    FROM context69.task_items item
    WHERE item.id = $1
      AND item.execution_task_id = $2
      AND item.execution_generation = $3
      AND item.task_id = $4
      AND item.status = 'running'
    FOR UPDATE
),
interrupted AS (
    UPDATE context69.task_attempts attempt
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        finished_at = now()
    FROM target
    WHERE attempt.item_id = target.id
      AND attempt.finished_at IS NULL
    RETURNING attempt.id
),
requeued AS (
    UPDATE context69.task_items item
    SET status = CASE WHEN target.attempt_count < 5 THEN 'queued' ELSE 'failed' END,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM target
    WHERE item.id = target.id
    RETURNING item.task_id
),
released_parent AS (
    -- The worker of the crashed item is gone, so the slot the parent held is
    -- released: the parent slot is a projection of the items that run.
    UPDATE context69.tasks task
    SET lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    WHERE task.id = (SELECT task_id FROM requeued)
      AND task.lease_token IS NOT NULL
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND (
          task.status IN ('queued', 'running')
          OR (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
      )
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items item
          WHERE item.task_id = task.id
            AND item.status = 'running'
            AND item.lease_until > clock_timestamp()
      )
    RETURNING task.id
)
SELECT task_id, (SELECT count(*) FROM released_parent) AS released_parent_leases
FROM requeued
