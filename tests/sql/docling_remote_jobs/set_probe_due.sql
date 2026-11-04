UPDATE context69.task_docling_remote_jobs
SET next_poll_at = $2,
    deadline_at = COALESCE($3, deadline_at),
    updated_at = now()
WHERE id = $1
