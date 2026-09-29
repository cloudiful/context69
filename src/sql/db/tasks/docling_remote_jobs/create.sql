INSERT INTO context69.task_docling_remote_jobs (
    task_id,
    item_id,
    provider,
    remote_task_id,
    remote_status,
    next_poll_at,
    deadline_at
)
VALUES (
    $1,
    $2,
    'docling',
    $3,
    $4,
    COALESCE($5, now()),
    $6
)
RETURNING
    id,
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    remote_status,
    attempt_count,
    next_poll_at,
    last_polled_at,
    deadline_at,
    lease_token,
    lease_until,
    last_error,
    created_at,
    updated_at,
    finished_at
