-- list_missing_document_versions.sql (issue 139, phase 3)
--
-- Bounded read-only audit page for documents whose current
-- `documents.record_hash` has no matching `document_versions` row.
-- Pages deterministically by document id (`$1` is the last id from the
-- previous page, NULL for the first page) with a caller-supplied `LIMIT`
-- (`$2`). SELECT-only; never mutates.
SELECT d.id,
       d.record_hash,
       d.title,
       d.summary,
       d.source_uri,
       d.metadata_json
FROM context69.documents d
WHERE ($1::BIGINT IS NULL OR d.id > $1)
  AND NOT EXISTS (
      SELECT 1
      FROM context69.document_versions v
      WHERE v.document_id = d.id
        AND v.record_hash = d.record_hash
  )
ORDER BY d.id ASC
LIMIT $2
