WITH target AS (
    SELECT item.id, item.task_id
    FROM context69.task_items item
    WHERE item.id = $1 AND item.task_id = $2
      AND item.execution_task_id = $3 AND item.execution_generation = $4
      AND item.status = 'running' AND item.lease_token = $5
), sealed AS (
    UPDATE context69.task_attempts attempt
    SET status = CASE WHEN $7 = 'cancelled' THEN 'cancelled' ELSE 'released' END,
        failure_stage = NULL, error_message = NULL, finished_at = now()
    FROM target
    WHERE attempt.item_id = target.id AND attempt.finished_at IS NULL
      AND attempt.execution_attempt_id = $6
    RETURNING attempt.item_id
), updated_item AS (
    UPDATE context69.task_items item
    SET status = CASE WHEN $7 = 'cancelled' THEN 'cancelled' ELSE 'queued' END,
        released_attempts = item.released_attempts + CASE WHEN $7 = 'cancelled' THEN 0 ELSE 1 END,
        waiting_reason = NULL, dependency_key = NULL, waiting_since = NULL,
        next_attempt_at = NULL, lease_token = NULL, lease_until = NULL,
        finished_at = CASE WHEN $7 = 'cancelled' THEN now() ELSE NULL END,
        updated_at = now()
    FROM target JOIN sealed ON sealed.item_id = target.id
    WHERE item.id = target.id
    RETURNING item.task_id
)
UPDATE context69.tasks task
SET pause_state = CASE WHEN $7 = 'cancelled' THEN 'unpaused' ELSE $8 END,
    paused_at = CASE WHEN $7 = 'cancelled' THEN NULL WHEN $8 = 'paused' THEN COALESCE(task.paused_at, now()) ELSE task.paused_at END,
    lease_token = CASE WHEN NOT EXISTS (SELECT 1 FROM context69.task_items other WHERE other.task_id = task.id AND other.id <> $1 AND other.status = 'running' AND other.lease_until > clock_timestamp()) THEN NULL ELSE task.lease_token END,
    lease_until = CASE WHEN NOT EXISTS (SELECT 1 FROM context69.task_items other WHERE other.task_id = task.id AND other.id <> $1 AND other.status = 'running' AND other.lease_until > clock_timestamp()) THEN NULL ELSE task.lease_until END,
    updated_at = now()
FROM updated_item WHERE task.id = updated_item.task_id
