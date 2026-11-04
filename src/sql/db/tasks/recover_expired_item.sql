-- The domain projection of one sealed execution segment (issue 1020 P2).
--
-- The library sealed the segment of an expired attempt. The domain attempt is
-- interrupted, the mirror of the lease is cleared, and the item is requeued
-- right away under the remaining domain attempt budget - a crashed worker is
-- not a provider failure, so it carries no backoff. The guard is the segment
-- itself: the library task and the execution generation, so a sealed segment of
-- an older generation never touches the item that replaced it.
--
-- A segment that had persisted a Docling dispatch intent is the one exception
-- (issue 1172 P5A): the interrupted `submitting` row becomes a durable
-- `submission_uncertain` outcome and the item parks on `waiting/docling` with
-- no due time and an unchanged failure budget, so no automatic POST can ever
-- follow an unknown dispatch outcome.
--
-- The parent holds no admission lease (issue 1172 P2): the library queue policy
-- owns execution capacity, so the recovery only projects the item and lets the
-- caller recompute the parent.
--
-- `$1` is the item, `$2` the library task of the segment, `$3` the execution
-- generation of the segment and `$4` the domain parent task the segment was
-- produced for.
WITH target AS (
    SELECT item.id, item.task_id, item.execution_failure_count
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
uncertain AS (
    UPDATE context69.task_docling_remote_jobs job
    SET status = 'submission_uncertain',
        remote_status = COALESCE(job.remote_status, job.status),
        last_error = COALESCE(
            job.last_error,
            'docling dispatch outcome is unknown after an interrupted submission'
        ),
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM target
    WHERE job.item_id = target.id
      AND job.status = 'submitting'
      AND job.submission_execution_task_id = $2
      AND job.submission_generation = $3
    RETURNING job.id
),
parked AS (
    UPDATE context69.task_items item
    SET status = 'waiting',
        failure_stage = NULL,
        error_message = 'docling dispatch outcome is unknown after an interrupted submission',
        waiting_reason = 'docling',
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = COALESCE(item.waiting_since, now()),
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM target
    WHERE item.id = target.id
      AND EXISTS (SELECT 1 FROM uncertain)
    RETURNING item.task_id
),
requeued AS (
    UPDATE context69.task_items item
    SET status = CASE
            WHEN target.execution_failure_count + 1 < 5 THEN 'queued'
            ELSE 'failed'
        END,
        execution_failure_count = LEAST(target.execution_failure_count + 1, 5),
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        retryable = target.execution_failure_count + 1 < 5,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM target
    WHERE item.id = target.id
      AND NOT EXISTS (SELECT 1 FROM uncertain)
    RETURNING item.task_id
)
SELECT task_id AS "task_id!" FROM parked
UNION ALL
SELECT task_id AS "task_id!" FROM requeued
