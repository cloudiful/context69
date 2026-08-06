UPDATE context69.library_files file
SET ingest_status = 'cancelled',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
WHERE file.ingest_status IN ('pending', 'running', 'failed', 'cancelled')
  AND file.id IN (
      SELECT DISTINCT item.file_id
      FROM context69.task_items item
      WHERE item.status = 'cancelled'
        AND item.file_id IS NOT NULL
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items other
      WHERE other.file_id = file.id
        AND other.status IN ('queued', 'running', 'waiting')
  )
