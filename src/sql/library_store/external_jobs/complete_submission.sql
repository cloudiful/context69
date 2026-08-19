UPDATE context69.task_external_jobs
SET remote_task_id = $2,
    status = 'pending',
    remote_status = NULL,
    last_polled_at = NULL,
    next_poll_at = $3,
    error_message = NULL,
    updated_at = now()
WHERE id = $1
  AND status = 'submitting'
RETURNING id, submission_count
