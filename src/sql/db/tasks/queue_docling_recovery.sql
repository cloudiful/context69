-- Atomically requeue a recoverable Docling item without any network request.
--
-- Queue-only recovery (issue #118 phase 4): unlike `recover_docling_item.sql`
-- (immediate recovery, which claims a real worker lease and bumps
-- `attempt_count` before POSTing a fresh remote job), this statement only
-- persists the item back to the `docling` scheduling queue with status
-- `queued`, lease and waiting fields cleared, and failure details reset for
-- the next attempt. No `task_attempts` row is inserted and `attempt_count`
-- is never touched, so a repeat call observes `already_queued` and can never
-- produce a new attempt or remote job. The dispatcher picks the item up
-- later under the persistent Docling admission ceiling, which is what makes
-- bulk recovery safe against submission floods.
--
-- Validation reuses the immediate-recovery boundaries: terminal tasks, active
-- leases, terminal items, live `pending`/`running` remote jobs, the Docling
-- dependency wait, and file presence all reject the request. Uncertain
-- `submitting` rows are never treated as remotely cancelled and reject with
-- `uncertain_submission`; quarantine them explicitly first.
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
               WHEN target.external_job_status = 'submitting'
                    THEN 'uncertain_submission'
               WHEN target.external_job_status IN ('pending', 'running')
                    AND (
                        target.external_job_deadline IS NULL
                        OR target.external_job_deadline > now()
                    )
                    THEN 'active_external_job'
               WHEN target.file_id IS NULL
                    THEN 'missing_file'
               WHEN target.status = 'queued'
                    AND target.stage = 'docling'
                    AND target.lease_token IS NULL
                    THEN 'already_queued'
               ELSE 'ok'
           END AS reason
    FROM requested_task
    LEFT JOIN target ON TRUE
), requeued AS (
    UPDATE context69.task_items item
    SET status = 'queued',
        stage = 'docling',
        lease_token = NULL,
        lease_until = NULL,
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
    RETURNING item.id, item.task_id, item.file_id
)
SELECT validation.task_id,
       validation.item_id,
       validation.file_id,
       validation.reason,
       target.remote_task_id,
       requeued.id AS requeued_item_id
FROM validation
LEFT JOIN target ON target.id = validation.item_id
LEFT JOIN requeued ON requeued.id = validation.item_id;
