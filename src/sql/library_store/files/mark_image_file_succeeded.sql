-- Promotes one image file to succeeded inside the publish transaction.
--
-- The update requires the exact file revision and the exact acknowledged
-- origin of the same publish attempt id and physical target. Because it runs
-- on the transaction that just marked that origin active, the file status and
-- origin acknowledgement commit together; a replacement cannot inherit the
-- old success and a different revision cannot be claimed by this attempt.
--
-- $1 file_id, $2 expected_sha256, $3 attempt_id, $4 endpoint,
-- $5 collection_name, $6 runtime_fingerprint.
UPDATE context69.library_files
SET ingest_status = 'succeeded',
    error_message = NULL,
    ingested_at = now(),
    updated_at = now()
WHERE id = $1
  AND sha256 = $2
  AND EXISTS (
      SELECT 1
      FROM context69.image_vector_origins origin
      WHERE origin.file_id = $1
        AND origin.content_sha256 = $2
        AND origin.attempt_id = $3
        AND origin.endpoint = $4
        AND origin.collection_name = $5
        AND origin.runtime_fingerprint = $6
        AND origin.state = 'active'
  )
