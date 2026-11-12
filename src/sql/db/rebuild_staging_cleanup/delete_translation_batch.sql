-- Delete one bounded batch of a queued target's staged translated chunks.
DELETE FROM context69.document_translation_rebuild_staging
WHERE target_collection = $1
  AND ctid IN (
      SELECT staged.ctid
      FROM context69.document_translation_rebuild_staging staged
      WHERE staged.target_collection = $1
      LIMIT $2
  )
