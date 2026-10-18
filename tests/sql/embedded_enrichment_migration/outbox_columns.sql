SELECT column_name AS "column_name!"
FROM information_schema.columns
WHERE table_schema = 'context69'
  AND table_name = 'document_metadata_payload_sync'
ORDER BY column_name
