SELECT
    id,
    status,
    remote_task_id,
    dispatch_attempted,
    submission_execution_task_id,
    submission_generation,
    submission_identity,
    last_error
FROM context69.task_docling_remote_jobs
WHERE id = $1
