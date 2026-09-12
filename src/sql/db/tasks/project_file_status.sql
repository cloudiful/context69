-- Project a terminal task item status onto its library file. The file row is
-- the business fact while the task item is execution detail, so this keeps
-- them consistent without ever regressing a file that already succeeded and
-- without stealing a file that still has another active item.
--
-- Referenced file is derived from the item itself; a NULL file_id or a
-- delete-batch item whose row is gone is a no-op.
UPDATE context69.library_files file
SET ingest_status = CASE
        WHEN file.ingest_status = 'succeeded' THEN file.ingest_status
        WHEN EXISTS (
            SELECT 1
            FROM context69.task_items sibling
            WHERE sibling.file_id = file.id
              AND sibling.id <> $1
              AND sibling.status IN ('queued', 'running', 'waiting')
        ) THEN file.ingest_status
        WHEN $2 = 'succeeded' THEN 'succeeded'
        WHEN $2 = 'failed' THEN 'failed'
        ELSE file.ingest_status
    END,
    error_message = CASE
        WHEN file.ingest_status = 'succeeded' THEN file.error_message
        WHEN EXISTS (
            SELECT 1
            FROM context69.task_items sibling
            WHERE sibling.file_id = file.id
              AND sibling.id <> $1
              AND sibling.status IN ('queued', 'running', 'waiting')
        ) THEN file.error_message
        WHEN $2 = 'succeeded' THEN NULL
        WHEN $2 = 'failed' THEN $3
        ELSE file.error_message
    END,
    ingested_at = CASE
        WHEN file.ingest_status = 'succeeded' THEN file.ingested_at
        WHEN EXISTS (
            SELECT 1
            FROM context69.task_items sibling
            WHERE sibling.file_id = file.id
              AND sibling.id <> $1
              AND sibling.status IN ('queued', 'running', 'waiting')
        ) THEN file.ingested_at
        WHEN $2 = 'succeeded' THEN now()
        WHEN $2 = 'failed' THEN NULL
        ELSE file.ingested_at
    END,
    updated_at = now()
WHERE file.id = (
    SELECT item.file_id
    FROM context69.task_items item
    JOIN context69.tasks task ON task.id = item.task_id
    WHERE item.id = $1
      AND item.file_id IS NOT NULL
      AND item.status = $2
      -- Defense in depth: never project a task's status onto a file owned by
      -- a different group, even if a legacy item references a foreign file.
      AND task.group_id = file.group_id
)
