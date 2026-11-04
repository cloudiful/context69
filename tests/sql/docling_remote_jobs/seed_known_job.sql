INSERT INTO context69.task_docling_remote_jobs (
    id,
    task_id,
    item_id,
    remote_task_id,
    remote_status,
    status,
    attempt_count,
    poll_step,
    next_poll_at,
    deadline_at,
    endpoint_base_url
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    'pending',
    $6,
    $7,
    $8,
    $9,
    $10
)
RETURNING id
