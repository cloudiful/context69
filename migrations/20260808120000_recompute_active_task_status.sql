-- Recompute active parent-task aggregates from their items.
--
-- The previous claim_items.sql marked every parent task 'running' as soon as
-- any item was claimable, even when the dispatcher only claimed a subset of
-- items (or none). That left a queue full of 'running' tasks whose items were
-- still 'queued', so the UI showed "running 0/1" for work that had never
-- started. This recomputes each active task's counters, status, stage, and
-- error fields from its items, following the runtime recompute.sql. Unlike
-- recompute.sql, started_at is cleared for active tasks with no running item
-- so spuriously pre-activated tasks do not claim a false start time; the
-- cancelled-preserving branch is omitted because only active tasks are touched.

WITH counts AS (
    SELECT
        task_id,
        count(*) FILTER (WHERE status = 'queued')::bigint AS queued_count,
        count(*) FILTER (WHERE status = 'running')::bigint AS running_count,
        count(*) FILTER (WHERE status = 'waiting')::bigint AS waiting_count,
        count(*) FILTER (WHERE status = 'succeeded')::bigint AS succeeded_count,
        count(*) FILTER (WHERE status = 'failed')::bigint AS failed_count,
        count(*) FILTER (WHERE status = 'cancelled')::bigint AS cancelled_count
    FROM context69.task_items
    GROUP BY task_id
)
UPDATE context69.tasks t
SET queued_count = counts.queued_count,
    running_count = counts.running_count,
    waiting_count = counts.waiting_count,
    succeeded_count = counts.succeeded_count,
    failed_count = counts.failed_count,
    cancelled_count = counts.cancelled_count,
    failure_stage = (
        SELECT failure_stage
        FROM context69.task_items
        WHERE task_id = t.id AND status = 'failed' AND failure_stage IS NOT NULL
        ORDER BY ordinal
        LIMIT 1
    ),
    error_summary = (
        SELECT error_message
        FROM context69.task_items
        WHERE task_id = t.id AND status = 'failed' AND error_message IS NOT NULL
        ORDER BY ordinal
        LIMIT 1
    ),
    stage = current_item.stage,
    waiting_reason = current_item.waiting_reason,
    dependency_key = current_item.dependency_key,
    next_attempt_at = current_item.next_attempt_at,
    lease_token = CASE
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
        THEN NULL
        ELSE t.lease_token
    END,
    lease_until = CASE
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
        THEN NULL
        ELSE t.lease_until
    END,
    status = CASE
        WHEN counts.cancelled_count = t.total_count THEN 'cancelled'
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
             AND counts.failed_count = 0 THEN 'succeeded'
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
             THEN 'failed'
        WHEN counts.running_count > 0 THEN 'running'
        WHEN counts.queued_count > 0 THEN 'queued'
        WHEN counts.waiting_count > 0 THEN 'waiting'
        ELSE 'queued'
    END,
    started_at = CASE
        WHEN counts.running_count > 0 THEN coalesce(t.started_at, now())
        ELSE NULL
    END,
    finished_at = CASE
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
        THEN now()
        ELSE NULL
    END,
    updated_at = now()
FROM counts
LEFT JOIN LATERAL (
    SELECT stage, waiting_reason, dependency_key, next_attempt_at
    FROM context69.task_items
    WHERE task_id = counts.task_id
      AND status IN ('queued', 'running', 'waiting')
    ORDER BY
        CASE status
            WHEN 'queued' THEN 0
            WHEN 'running' THEN 1
            ELSE 2
        END,
        next_attempt_at NULLS FIRST,
        ordinal
    LIMIT 1
) current_item ON TRUE
WHERE t.id = counts.task_id
  AND t.status IN ('queued', 'running', 'waiting');
