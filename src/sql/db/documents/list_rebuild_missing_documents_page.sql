-- Bounded page of current documents without a canonical version body.
--
-- The rebuild fails closed on any row: it never silently skips a document or
-- fabricates a body from stale chunks.
SELECT d.id AS document_id
FROM context69.documents d
WHERE ($1::BIGINT IS NULL OR d.id > $1)
  AND NOT EXISTS (
      SELECT 1
      FROM context69.document_versions v
      WHERE v.document_id = d.id
        AND v.record_hash = d.record_hash
  )
ORDER BY d.id
LIMIT $2
