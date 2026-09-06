-- get_file_library_document_version_for_update.sql (issue 139, phase 4)
--
-- Lock the current `file_library` document row inside the per-document
-- backfill transaction and re-read the fields needed for the complete
-- snapshot. The `source_key = 'file_library'` predicate is hardcoded so
-- non-file_library ids return no row. Call inside a transaction; `FOR
-- UPDATE` prevents a concurrent business-fields publish from racing the
-- snapshot. SELECT-only apart from the row lock; never mutates.
SELECT id,
       record_hash,
       title,
       summary,
       source_uri,
       published_at,
       metadata_json
FROM context69.documents
WHERE id = $1
  AND source_key = 'file_library'
FOR UPDATE
