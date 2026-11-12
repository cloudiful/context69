-- Install exactly one staged original page into the live chunk table.
--
-- The page is selected from the immutable staging keyset, never from a
-- `RETURNING` clause or a join that could add rows. A repeated page is
-- idempotent: the staging chunk id is the live primary key.
INSERT INTO context69.document_chunks (
    id,
    document_id,
    chunk_index,
    chunk_text,
    record_hash
)
SELECT
    s.chunk_id,
    s.document_id,
    s.chunk_index,
    s.chunk_text,
    s.record_hash
FROM context69.document_chunk_rebuild_staging s
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
ON CONFLICT (id) DO NOTHING
