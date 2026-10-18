SELECT indexdef AS "indexdef!"
FROM pg_indexes
WHERE schemaname = 'context69'
  AND tablename = 'document_metadata_payload_sync'
  AND indexname = 'uq_document_metadata_payload_sync_pending'
