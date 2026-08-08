UPDATE context69.task_external_jobs
SET status = $2,
    remote_status = $3,
    last_polled_at = now(),
    next_poll_at = $4,
    error_message = $5,
    updated_at = now()
WHERE id = $1
