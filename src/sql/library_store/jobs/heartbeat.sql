UPDATE context69.library_ingest_jobs
SET lease_expires_at = now() + ($3::BIGINT * INTERVAL '1 second'),
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
RETURNING id
