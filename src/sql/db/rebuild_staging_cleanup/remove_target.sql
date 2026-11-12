-- Drop the address row only when every staging row family of its target is
-- empty, so remaining rows stay the progress of a later tick.
DELETE FROM context69.document_rebuild_staging_cleanup cleanup
WHERE cleanup.target_collection = $1
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
  AND NOT EXISTS (
      SELECT 1
      FROM context69.document_rebuild_staging_documents document
      WHERE document.target_collection = $1
  )
