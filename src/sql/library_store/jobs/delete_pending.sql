DELETE FROM context69.library_ingest_jobs
WHERE id = $1
  AND status = 'pending'
  AND lease_token IS NULL
RETURNING id
