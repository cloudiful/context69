-- Atomically claim a Docling item for administrator recovery.
--
-- The item remains protected by a real task lease while the caller performs
-- the external submission. This prevents both the dispatcher and a second
-- recovery request from creating another remote job concurrently.
WITH requested_task AS (
    SELECT id, status
    FROM context69.tasks
    WHERE id = $1
    FOR UPDATE
), target AS (
    SELECT item.id,
           item.task_id,
           item.file_id,
           item.stage,
           item.status,
           item.waiting_reason,
           item.dependency_key,
           item.failure_stage,
           item.lease_token,
           item.lease_until,
           external_job.status AS external_job_status,
           external_job.deadline_at AS external_job_deadline,
            external_job.remote_task_id,
            EXISTS (
                SELECT 1
                FROM context69.task_docling_recovery_audit audit
                WHERE audit.task_id = item.task_id
                  AND audit.item_id = item.id
            ) AS has_recovery_audit
    FROM context69.task_items item
    JOIN requested_task task ON task.id = item.task_id
    LEFT JOIN LATERAL (
        SELECT job.status, job.deadline_at, job.remote_task_id
        FROM context69.task_external_jobs job
        WHERE job.item_id = item.id
          AND job.provider = 'docling'
        ORDER BY job.submitted_at DESC, job.created_at DESC
        LIMIT 1
    ) external_job ON TRUE
    WHERE item.task_id = $1
      AND item.stage IN ('docling', 'docling_poll')
    ORDER BY item.ordinal
    LIMIT 1
    FOR UPDATE OF item
), validation AS (
    SELECT requested_task.id AS task_id,
           target.id AS item_id,
           target.file_id,
           CASE
               WHEN requested_task.status IN ('succeeded', 'cancelled')
                   THEN 'task_terminal'
               WHEN target.id IS NULL
                   THEN 'no_docling_item'
               WHEN requested_task.status = 'failed'
                    AND target.status = 'failed'
                    AND target.failure_stage IN ('docling', 'docling_poll')
                    AND (target.lease_until IS NULL OR target.lease_until <= now())
                   THEN 'ok'
               WHEN target.status = 'running'
                    AND target.stage IN ('docling', 'docling_poll')
                    AND (target.lease_until IS NULL OR target.lease_until <= now())
                   THEN 'ok'
               WHEN target.lease_until > now()
                   THEN 'lease_active'
               WHEN target.status NOT IN ('queued', 'waiting')
                   THEN 'item_terminal'
               WHEN target.stage = 'docling'
                    AND target.status = 'waiting'
                    AND target.waiting_reason = 'dependency'
                    AND target.dependency_key = 'docling'
                    THEN 'dependency_waiting'
               WHEN target.external_job_status IN ('pending', 'running')
                    AND target.has_recovery_audit
                    AND (
                        target.external_job_deadline IS NULL
                        OR target.external_job_deadline > now()
                    )
                    THEN 'already_recovered'
               WHEN target.external_job_status IN ('pending', 'running')
                    AND NOT target.has_recovery_audit
                    AND (
                        target.external_job_deadline IS NULL
                        OR target.external_job_deadline > now()
                    )
                    THEN 'active_external_job'
               ELSE 'ok'
           END AS reason
    FROM requested_task
    LEFT JOIN target ON TRUE
), claimed AS (
    UPDATE context69.task_items item
    SET status = 'running',
        stage = 'docling',
        attempt_count = item.attempt_count + 1,
        lease_token = $2,
        lease_until = now() + interval '5 minutes',
        started_at = coalesce(item.started_at, now()),
        finished_at = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM validation
    WHERE item.id = validation.item_id
      AND validation.reason = 'ok'
    RETURNING item.id, item.task_id, item.file_id, item.lease_token, item.attempt_count
), attempts AS (
    INSERT INTO context69.task_attempts (task_id, item_id, attempt, status)
    SELECT task_id, id, attempt_count, 'running'
    FROM claimed
    RETURNING item_id, id AS attempt_id
), activated AS (
    UPDATE context69.tasks task
    SET status = 'running',
        started_at = coalesce(task.started_at, now()),
        stage = 'docling',
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        updated_at = now()
    FROM claimed
    WHERE task.id = claimed.task_id
    RETURNING task.id
)
SELECT validation.task_id,
       validation.item_id,
       validation.file_id,
       validation.reason,
       target.remote_task_id,
       claimed.lease_token,
       attempts.attempt_id
FROM validation
LEFT JOIN target ON target.id = validation.item_id
LEFT JOIN claimed ON claimed.id = validation.item_id
LEFT JOIN attempts ON attempts.item_id = validation.item_id
LEFT JOIN activated ON activated.id = validation.task_id;
