UPDATE context69.document_metadata_payload_sync
SET next_attempt_at = now()
WHERE id = $1
