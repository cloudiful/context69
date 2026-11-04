-- Milliseconds until the dispatcher must probe a known Docling job again.
--
-- Only live parked Docling items with an active known job, a captured endpoint,
-- an active parent and failure budget left are candidates. The due instant is
-- the earliest of the item's own retry time, the job's next poll and its
-- deadline, all measured against one database-clock observation. An expired
-- deadline therefore wins even when the next poll is later. No candidate
-- returns NULL, which means "no wake": the dispatcher keeps its ordinary
-- periodic tick.
WITH observed AS (
    SELECT clock_timestamp() AS at
)
SELECT min(
    GREATEST(
        0,
        (
            EXTRACT(EPOCH FROM (
                LEAST(
                    COALESCE(item.next_attempt_at, observed.at),
                    job.next_poll_at,
                    COALESCE(job.deadline_at, job.next_poll_at)
                ) - observed.at
            )) * 1000
        )::bigint
    )
) AS "remaining_ms"
FROM context69.task_docling_remote_jobs AS job
JOIN context69.task_items AS item ON item.id = job.item_id
JOIN context69.tasks AS task ON task.id = job.task_id
CROSS JOIN observed
WHERE job.status IN ('pending', 'running')
  AND job.endpoint_base_url IS NOT NULL
  AND item.status = 'waiting'
  AND COALESCE(item.waiting_reason, '') = 'docling'
  AND item.execution_failure_count < 5
  AND task.status IN ('queued', 'running', 'waiting')
  AND task.deleted_at IS NULL
