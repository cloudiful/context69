WITH expired AS (
    UPDATE context69.task_attempts
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        finished_at = now()
    WHERE item_id = $1
      AND finished_at IS NULL
      AND EXISTS (
          SELECT 1
          FROM context69.task_items
          WHERE id = $1
            AND status = 'running'
            AND (lease_until IS NULL OR lease_until < now())
      )
), claimed AS (
    UPDATE context69.task_items AS item
    SET status = 'running',
        attempt_count = item.attempt_count + 1,
        lease_token = $2,
        lease_until = now() + interval '5 minutes',
        started_at = coalesce(item.started_at, now()),
        finished_at = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM context69.tasks AS task
    WHERE item.id = $1
      AND item.task_id = task.id
      AND task.status = 'running'
      AND (
          item.status IN ('queued', 'waiting')
          AND (item.next_attempt_at IS NULL OR item.next_attempt_at <= now())
          OR (
              item.status = 'running'
              AND (item.lease_until IS NULL OR item.lease_until < now())
          )
      )
    RETURNING item.id, item.task_id, item.attempt_count, item.lease_token,
              item.payload, item.file_id, item.stage
), attempt AS (
    INSERT INTO context69.task_attempts (task_id, item_id, attempt, status)
    SELECT task_id, id, attempt_count, 'running'
    FROM claimed
    RETURNING id AS attempt_id
)
SELECT claimed.id,
       claimed.task_id,
       claimed.attempt_count,
       claimed.lease_token AS "lease_token!",
       claimed.payload,
       claimed.file_id,
       claimed.stage,
       attempt.attempt_id
FROM claimed
CROSS JOIN attempt
