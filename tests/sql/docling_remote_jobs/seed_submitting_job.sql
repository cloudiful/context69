INSERT INTO context69.task_docling_remote_jobs (
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    dispatch_attempted,
    submission_execution_task_id,
    submission_generation,
    submission_identity,
    endpoint_base_url,
    deadline_at
)
VALUES (
    $1,
    $2,
    'docling',
    NULL,
    'submitting',
    $3,
    $4,
    $5,
    $6,
    $7,
    $8
)
RETURNING id
