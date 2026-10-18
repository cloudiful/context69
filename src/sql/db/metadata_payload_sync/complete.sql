UPDATE context69.document_metadata_payload_sync
SET completed_at = now(),
    lease_token = NULL,
    lease_expires_at = NULL,
    last_error = NULL,
    updated_at = now()
WHERE id = $1
  AND completed_at IS NULL
  AND lease_token = $2
