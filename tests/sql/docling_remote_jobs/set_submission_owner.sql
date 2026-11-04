UPDATE context69.task_docling_remote_jobs
SET submission_execution_task_id = $2,
    submission_generation = $3,
    updated_at = now()
WHERE id = $1
RETURNING id
