-- Opted-in, succeeded files whose source has not been released yet. Sync
-- control files are excluded: their source is managed by the sync engine and
-- releasing it would break the managed source.
SELECT file.id, file.group_id
FROM context69.library_files AS file
WHERE file.delete_source_after_processing
  AND file.source_released_at IS NULL
  AND file.ingest_status = 'succeeded'
  AND lower(file.filename) <> 'source.json'
  AND (file.external_id IS NULL OR file.external_id NOT LIKE 'source-folder:%')
ORDER BY file.id
LIMIT $1
