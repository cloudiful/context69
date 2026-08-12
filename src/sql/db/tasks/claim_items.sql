WITH eligible AS (
    SELECT ti.id, ti.task_id
    FROM context69.task_items ti
    JOIN context69.tasks task ON task.id = ti.task_id
    WHERE (
            task.status IN ('queued', 'running')
            OR (
                task.status = 'waiting'
                AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
            )
        )
      AND (
          (
              ti.status IN ('queued', 'waiting')
              AND ti.attempt_count < 5
              AND (ti.next_attempt_at IS NULL OR ti.next_attempt_at <= now())
          )
          OR (
              ti.status = 'running'
              AND (ti.lease_until IS NULL OR ti.lease_until < now())
          )
      )
    ORDER BY CASE
                 WHEN ti.waiting_reason = 'external_job' THEN 0
                 ELSE 1
             END,
             ti.created_at
    LIMIT $1
    FOR UPDATE OF ti SKIP LOCKED
), activated AS (
    UPDATE context69.tasks AS task
    SET status = 'running',
        started_at = coalesce(task.started_at, now()),
        updated_at = now()
    WHERE task.id IN (SELECT task_id FROM eligible)
      AND (
          task.status = 'queued'
          OR (
              task.status = 'waiting'
              AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
          )
      )
), exhausted AS (
    UPDATE context69.task_items AS item
    SET status = 'failed',
        failure_stage = 'attempts',
        error_message = 'exceeded maximum attempt count',
        lease_token = NULL,
        lease_until = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        finished_at = now(),
        updated_at = now()
    FROM context69.tasks AS task
    WHERE item.task_id = task.id
      AND (
          task.status IN ('queued', 'running')
          OR (
              task.status = 'waiting'
              AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
          )
      )
      AND item.status IN ('queued', 'waiting')
      AND item.attempt_count >= 5
      AND (item.next_attempt_at IS NULL OR item.next_attempt_at <= now())
    RETURNING item.task_id, item.id AS item_id, item.file_id
), exhausted_files AS (
    UPDATE context69.library_files AS file
    SET ingest_status = 'failed',
        error_message = 'exceeded maximum attempt count',
        updated_at = now()
    FROM exhausted
    WHERE file.id = exhausted.file_id
      AND exhausted.file_id IS NOT NULL
      AND file.ingest_status IN ('pending', 'running')
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items other
          WHERE other.file_id = file.id
            AND other.id <> exhausted.item_id
            AND other.status IN ('queued', 'running', 'waiting')
      )
), exhausted_tasks AS (
    UPDATE context69.tasks AS task
    SET status = 'failed',
        failure_stage = 'attempts',
        error_summary = 'task items exceeded the maximum attempt count',
        finished_at = now(),
        updated_at = now()
    FROM exhausted
    WHERE task.id = exhausted.task_id
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items ti
          WHERE ti.task_id = task.id
            AND ti.status IN ('queued', 'waiting')
            AND ti.attempt_count < 5
      )
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items ti
          WHERE ti.task_id = task.id
            AND ti.status = 'running'
      )
), expired AS (
    UPDATE context69.task_attempts AS attempt
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        finished_at = now()
    FROM eligible
    JOIN context69.task_items item ON item.id = eligible.id
    WHERE attempt.item_id = eligible.id
      AND item.status = 'running'
      AND (item.lease_until IS NULL OR item.lease_until < now())
      AND attempt.finished_at IS NULL
), claimed AS (
    UPDATE context69.task_items AS item
    SET status = 'running',
        attempt_count = item.attempt_count + 1,
        lease_token = gen_random_uuid(),
        lease_until = now() + interval '5 minutes',
        started_at = coalesce(item.started_at, now()),
        finished_at = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM eligible
    WHERE item.id = eligible.id
    RETURNING item.id, item.task_id, item.attempt_count, item.lease_token,
              item.payload, item.file_id, item.stage, item.input_storage_object_id
), attempts AS (
    INSERT INTO context69.task_attempts (task_id, item_id, attempt, status)
    SELECT task_id, id, attempt_count, 'running'
    FROM claimed
    RETURNING item_id, id AS attempt_id
)
SELECT claimed.id,
       claimed.task_id,
       claimed.attempt_count,
       claimed.lease_token AS "lease_token!",
       claimed.payload,
       claimed.file_id,
       claimed.stage,
       claimed.input_storage_object_id,
       attempts.attempt_id,
       task.kind,
       task.group_id,
       task.group_path,
       task.source_key
FROM claimed
JOIN attempts ON attempts.item_id = claimed.id
JOIN context69.tasks task ON task.id = claimed.task_id
