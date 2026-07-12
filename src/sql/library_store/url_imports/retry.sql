UPDATE context69.library_url_import_jobs
SET status = 'queued',
    error_code = NULL,
    error_message = NULL,
    finished_at = NULL,
    updated_at = now()
WHERE group_id = $1 AND id = $2 AND status = 'failed'
RETURNING *
