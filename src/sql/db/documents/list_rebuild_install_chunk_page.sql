-- One immutable keyset page of the staged original chunks to install.
--
-- The keyset is `(document_id, chunk_index)`, the exact primary key of the
-- staging table, so a page is stable while the installation drains it. `$2`/
-- `$3` are the exclusive lower bound (the committed install cursor); the
-- returned last key bounds the same page for the write and the proofs.
SELECT
    s.document_id,
    s.chunk_index,
    s.chunk_id
FROM context69.document_chunk_rebuild_staging s
WHERE s.target_collection = $1
  AND (
      $2::bigint IS NULL
      OR s.document_id > $2
      OR (s.document_id = $2 AND s.chunk_index > $3)
  )
ORDER BY s.document_id, s.chunk_index
LIMIT $4
