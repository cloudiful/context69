INSERT INTO context69.task_external_jobs (
    item_id,
    provider,
    remote_task_id,
    status,
    submitted_at,
    next_poll_at,
    deadline_at,
    updated_at
)
VALUES ($1, $2, $3, $4, now(), $5, $6, now())
ON CONFLICT (item_id, provider)
DO UPDATE SET
    remote_task_id = EXCLUDED.remote_task_id,
    status = EXCLUDED.status,
    remote_status = NULL,
    submitted_at = now(),
    last_polled_at = NULL,
    next_poll_at = EXCLUDED.next_poll_at,
    deadline_at = EXCLUDED.deadline_at,
    error_message = NULL,
    updated_at = now()
