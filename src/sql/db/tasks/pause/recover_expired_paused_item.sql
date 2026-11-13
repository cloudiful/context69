-- Projects one natively recovered pause checkpoint into the domain.
--
-- The library sealed the expired attempt as paused (not as a failure) and
-- requeued the task with its paused counter advanced. The domain mirrors that
-- exact state: the item is requeued without spending the failure budget, the
-- open domain attempt is sealed paused, and the parent pause survives.
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
    SET status = 'paused',
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
    SET status = 'queued',
        paused_attempts = item.paused_attempts + 1,
        waiting_reason = NULL,
        dependency_key = NULL,
        waiting_since = NULL,
        next_attempt_at = NULL,
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM target
    WHERE item.id = target.id
    RETURNING item.task_id, item.id
)
UPDATE context69.tasks task
SET pause_state = 'paused',
    paused_at = COALESCE(task.paused_at, now()),
    lease_token = CASE
        WHEN NOT EXISTS (
            SELECT 1
            FROM context69.task_items other
            WHERE other.task_id = task.id
              AND other.id <> $1
              AND other.status = 'running'
              AND EXISTS (
                  SELECT 1
                  FROM cloudiful_tasks.attempts attempt
                  JOIN cloudiful_tasks.tasks library_task
                    ON library_task.id = attempt.task_id
                  WHERE attempt.task_id = other.execution_task_id
                    AND attempt.fencing_token = other.lease_token
                    AND attempt.status = 'running'
                    AND attempt.lease_expires_at > clock_timestamp()
                    AND library_task.namespace = 'context69'
                    AND library_task.queue = 'items'
                    AND library_task.kind = 'context69.item'
              )
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
              AND EXISTS (
                  SELECT 1
                  FROM cloudiful_tasks.attempts attempt
                  JOIN cloudiful_tasks.tasks library_task
                    ON library_task.id = attempt.task_id
                  WHERE attempt.task_id = other.execution_task_id
                    AND attempt.fencing_token = other.lease_token
                    AND attempt.status = 'running'
                    AND attempt.lease_expires_at > clock_timestamp()
                    AND library_task.namespace = 'context69'
                    AND library_task.queue = 'items'
                    AND library_task.kind = 'context69.item'
              )
        ) THEN NULL
        ELSE task.lease_until
    END,
    updated_at = now()
FROM updated_item
WHERE task.id = updated_item.task_id
