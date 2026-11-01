-- update_library_chunk_record_hash.sql (issue 1138, P19)
--
-- Rewrite the record hash of the chunks of one document whose hash diverged.
-- Only the rows of the document that passed the business-fields CAS are
-- touched, so a CAS miss never rewrites a later generation's chunks.
UPDATE context69.document_chunks
SET record_hash = $2
WHERE document_id = $1
  AND record_hash IS DISTINCT FROM $2
