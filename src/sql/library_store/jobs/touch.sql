UPDATE context69.library_ingest_jobs
SET updated_at = now()
WHERE id = $1
  AND status = 'running'
RETURNING id
