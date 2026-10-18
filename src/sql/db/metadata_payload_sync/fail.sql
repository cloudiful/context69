UPDATE context69.document_metadata_payload_sync
SET attempts = attempts + 1,
    next_attempt_at = now() + make_interval(secs => ($3)::int),
    last_error = $4,
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE id = $1
  AND completed_at IS NULL
  AND lease_token = $2
