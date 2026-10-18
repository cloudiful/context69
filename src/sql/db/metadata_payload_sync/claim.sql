WITH due AS (
    SELECT id
    FROM context69.document_metadata_payload_sync
    WHERE completed_at IS NULL
      AND next_attempt_at <= now()
      AND (lease_expires_at IS NULL OR lease_expires_at <= now())
    ORDER BY next_attempt_at, id
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
UPDATE context69.document_metadata_payload_sync sync
SET lease_token = $1,
    lease_expires_at = now() + make_interval(secs => ($2)::int),
    updated_at = now()
FROM due
WHERE sync.id = due.id
RETURNING sync.id, sync.document_id, sync.payload_key, sync.attempts, sync.lease_token
