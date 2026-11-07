-- Projects one committed native park into the domain.
--
-- The item and its open domain attempt are sealed under the captured segment
-- identity. A park that raced a cancellation follows the native cancelled
-- outcome and never projects a pause. The attempt counters stay untouched and
-- only a genuine park adds one parked attempt, so the effective failure usage
-- is `attempt_count - paused_attempts`.
WITH target AS (
    SELECT item.id, item.task_id
    FROM context69.task_items item
    WHERE item.id = $1
      AND item.task_id = $2
      AND item.execution_task_id = $3
      AND item.execution_generation = $4
      AND item.status = 'running'
),
sealed AS (
    UPDATE context69.task_attempts attempt
    SET status = CASE WHEN $5 = 'cancelled' THEN 'cancelled' ELSE 'paused' END,
        failure_stage = NULL,
        error_message = NULL,
        finished_at = now()
    FROM target
    WHERE attempt.item_id = target.id
      AND attempt.finished_at IS NULL
    RETURNING attempt.item_id
),
updated_item AS (
    UPDATE context69.task_items item
    SET status = CASE WHEN $5 = 'cancelled' THEN 'cancelled' ELSE 'queued' END,
        paused_attempts = item.paused_attempts
            + CASE WHEN $5 = 'cancelled' THEN 0 ELSE 1 END,
        waiting_reason = NULL,
        dependency_key = NULL,
        waiting_since = NULL,
        next_attempt_at = NULL,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = CASE WHEN $5 = 'cancelled' THEN now() ELSE NULL END,
        updated_at = now()
    FROM target
    WHERE item.id = target.id
    RETURNING item.task_id, item.id
)
UPDATE context69.tasks task
SET pause_state = CASE WHEN $5 = 'cancelled' THEN 'unpaused' ELSE $6 END,
    paused_at = CASE
        WHEN $5 = 'cancelled' THEN NULL
        WHEN $6 = 'paused' THEN COALESCE(task.paused_at, now())
        ELSE task.paused_at
    END,
    lease_token = CASE
        WHEN NOT EXISTS (
            SELECT 1
            FROM context69.task_items other
            WHERE other.task_id = task.id
              AND other.id <> $1
              AND other.status = 'running'
              AND other.lease_until > clock_timestamp()
        ) THEN NULL
        ELSE task.lease_token
    END,
    lease_until = CASE
        WHEN NOT EXISTS (
            SELECT 1
            FROM context69.task_items other
            WHERE other.task_id = task.id
              AND other.id <> $1
              AND other.status = 'running'
              AND other.lease_until > clock_timestamp()
        ) THEN NULL
        ELSE task.lease_until
    END,
    updated_at = now()
FROM updated_item
WHERE task.id = updated_item.task_id
