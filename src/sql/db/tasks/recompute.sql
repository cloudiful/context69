UPDATE context69.tasks t
SET queued_count = counts.queued_count,
    running_count = counts.running_count,
    succeeded_count = counts.succeeded_count,
    failed_count = counts.failed_count,
    cancelled_count = counts.cancelled_count,
    failure_stage = (
        SELECT failure_stage
        FROM context69.task_items
        WHERE task_id = $1 AND status = 'failed' AND failure_stage IS NOT NULL
        ORDER BY ordinal
        LIMIT 1
    ),
    error_summary = (
        SELECT error_message
        FROM context69.task_items
        WHERE task_id = $1 AND status = 'failed' AND error_message IS NOT NULL
        ORDER BY ordinal
        LIMIT 1
    ),
    lease_token = CASE WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count THEN NULL ELSE t.lease_token END,
    lease_until = CASE WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count THEN NULL ELSE t.lease_until END,
    status = CASE
        WHEN t.status = 'cancelled' THEN 'cancelled'
        WHEN counts.cancelled_count = t.total_count THEN 'cancelled'
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
             AND counts.failed_count = 0 THEN 'succeeded'
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
             THEN 'failed'
        ELSE 'running'
    END,
    finished_at = CASE
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
        THEN now()
        ELSE t.finished_at
    END,
    updated_at = now()
FROM (
    SELECT
        task_id,
        count(*) FILTER (WHERE status = 'queued')::bigint AS queued_count,
        count(*) FILTER (WHERE status = 'running')::bigint AS running_count,
        count(*) FILTER (WHERE status = 'succeeded')::bigint AS succeeded_count,
        count(*) FILTER (WHERE status = 'failed')::bigint AS failed_count,
        count(*) FILTER (WHERE status = 'cancelled')::bigint AS cancelled_count
    FROM context69.task_items
    WHERE task_id = $1
    GROUP BY task_id
) counts
WHERE t.id = counts.task_id
