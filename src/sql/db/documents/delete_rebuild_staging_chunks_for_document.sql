-- Clear one document's staged chunks before it is re-staged on resume.
DELETE FROM context69.document_chunk_rebuild_staging
WHERE target_collection = $1
  AND document_id = $2
