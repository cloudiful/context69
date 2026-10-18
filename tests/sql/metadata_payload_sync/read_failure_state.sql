SELECT attempts,
       last_error,
       next_attempt_at > now() AS "next_due!"
FROM context69.document_metadata_payload_sync
WHERE id = $1
