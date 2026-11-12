-- Fail-closed proof of one installed original page.
--
-- The expected rows come from the staging source of the same immutable
-- keyset, and every field the live row must carry is compared. A nonzero
-- result aborts the batch transaction before its cursor commits.
SELECT count(*)::bigint AS "mismatch!"
FROM context69.document_chunk_rebuild_staging s
LEFT JOIN context69.document_chunks c ON c.id = s.chunk_id
WHERE s.target_collection = $1
  AND (
      $2::bigint IS NULL
      OR s.document_id > $2
      OR (s.document_id = $2 AND s.chunk_index > $3)
  )
  AND (
      s.document_id < $4
      OR (s.document_id = $4 AND s.chunk_index <= $5)
  )
  AND (
      c.id IS NULL
      OR c.document_id <> s.document_id
      OR c.chunk_index <> s.chunk_index
      OR c.chunk_text <> s.chunk_text
      OR c.record_hash <> s.record_hash
  )
