UPDATE context69.library_url_import_jobs
SET status = 'ingesting',
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status IN ('downloading', 'ingesting')
RETURNING id
