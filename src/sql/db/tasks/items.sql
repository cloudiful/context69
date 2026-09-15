-- Paged task items for `GET /v1/tasks/{task_id}/items` (issue 413 Phase 1).
--
-- `$4::text` narrows to one item status; NULL lists every status.
-- Fixed active-first ordering (failed, running, queued, waiting,
-- cancelled, succeeded, then ordinal) pins in-flight/failed to the top and
-- sinks succeeded to the bottom. `cursor`/`offset` ($3) is scoped to the
-- current status filter: clients reset to 0 when the filter changes, then
-- follow `next_cursor` until NULL to page through the full filtered set.
SELECT item.id,
       item.task_id,
       item.ordinal,
       item.status,
       item.resource_id,
       item.file_id,
       item.stage,
       item.waiting_reason,
       item.dependency_key,
       item.next_attempt_at,
       item.failure_stage,
       item.error_message,
       item.attempt_count,
       item.retryable,
       item.created_at,
       item.started_at,
       item.finished_at,
       job.provider AS external_job_provider,
       job.remote_task_id AS external_job_remote_task_id,
       job.status AS external_job_status,
       job.remote_status AS external_job_remote_status,
       job.submitted_at AS external_job_submitted_at,
       job.last_polled_at AS external_job_last_polled_at,
       job.next_poll_at AS external_job_next_poll_at,
       job.deadline_at AS external_job_deadline_at,
       job.error_message AS external_job_error_message
FROM context69.task_items item
LEFT JOIN LATERAL (
     SELECT job.provider,
            job.remote_task_id,
            job.status,
            job.remote_status,
            job.submitted_at,
            job.last_polled_at,
            job.next_poll_at,
            job.deadline_at,
            job.error_message
     FROM context69.task_external_jobs job
     WHERE job.item_id = item.id
       AND job.provider = 'docling'
     ORDER BY job.submitted_at DESC, job.created_at DESC
     LIMIT 1
) job ON TRUE
WHERE item.task_id = $1
  AND ($4::text IS NULL OR item.status = $4::text)
ORDER BY
    CASE item.status
        WHEN 'failed' THEN 0
        WHEN 'running' THEN 1
        WHEN 'queued' THEN 2
        WHEN 'waiting' THEN 3
        WHEN 'cancelled' THEN 4
        WHEN 'succeeded' THEN 5
        ELSE 6
    END,
    item.ordinal
LIMIT $2 OFFSET $3
