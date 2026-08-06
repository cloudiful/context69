-- Restore per-task counters for every task the global cancel touched. Tasks
-- are terminal at this point, so a cancelled task only needs its item counts;
-- the status guard keeps already-consistent cancelled rows a no-op.
UPDATE context69.tasks task
SET queued_count = counts.queued_count,
    running_count = counts.running_count,
    waiting_count = counts.waiting_count,
    succeeded_count = counts.succeeded_count,
    failed_count = counts.failed_count,
    cancelled_count = counts.cancelled_count,
    stage = NULL,
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = NULL,
    lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
FROM (
    SELECT task_id,
        count(*) FILTER (WHERE status = 'queued')::BIGINT AS queued_count,
        count(*) FILTER (WHERE status = 'running')::BIGINT AS running_count,
        count(*) FILTER (WHERE status = 'waiting')::BIGINT AS waiting_count,
        count(*) FILTER (WHERE status = 'succeeded')::BIGINT AS succeeded_count,
        count(*) FILTER (WHERE status = 'failed')::BIGINT AS failed_count,
        count(*) FILTER (WHERE status = 'cancelled')::BIGINT AS cancelled_count
    FROM context69.task_items
    GROUP BY task_id
) counts
WHERE task.id = counts.task_id
  AND task.status = 'cancelled'
