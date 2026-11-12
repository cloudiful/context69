-- Delete one bounded batch of a queued target's staged document metadata.
--
-- Only reached once the staged original and translation children are empty, so
-- the metadata rows cannot disappear ahead of the rows that describe them.
DELETE FROM context69.document_rebuild_staging_documents
WHERE target_collection = $1
  AND ctid IN (
      SELECT staged.ctid
      FROM context69.document_rebuild_staging_documents staged
      WHERE staged.target_collection = $1
      LIMIT $2
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.document_chunk_rebuild_staging original
      WHERE original.target_collection = $1
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.document_translation_rebuild_staging translated
      WHERE translated.target_collection = $1
  )
