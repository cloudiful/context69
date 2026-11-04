-- Load the one unresolved Docling submission of an item (issue 1172 P5A).
--
-- "Unresolved" spans the durable intent, the unknown outcome and the two
-- known-id observation states, so one read tells the worker whether it must
-- adopt a known job, park an unknown outcome, or begin a fresh intent. The
-- unresolved unique index guarantees at most one row.
SELECT
    id,
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    remote_status,
    attempt_count,
    poll_step,
    next_poll_at,
    clock_timestamp() AS "observed_at!",
    last_polled_at,
    deadline_at,
    endpoint_base_url,
    lease_token,
    lease_until,
    last_error,
    created_at,
    updated_at,
    finished_at,
    dispatch_attempted,
    submission_execution_task_id,
    submission_generation,
    submission_identity
FROM context69.task_docling_remote_jobs
WHERE item_id = $1
  AND status IN ('submitting', 'submission_uncertain', 'pending', 'running')
ORDER BY created_at DESC
LIMIT 1
