-- Active processing item for each requested file in one group, used to
-- deduplicate file-ingest task submissions. Delete/translation tasks are
-- intentionally excluded: only ingesting task kinds own a file's processing
-- slot. Callers must already have validated that the files belong to the
-- group; this filter keeps the lookup from ever reusing another tenant's task.
SELECT item.task_id AS task_id,
       item.file_id AS file_id
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.file_id = ANY($1)
  AND task.group_id = $2
  AND item.status IN ('queued', 'running', 'waiting')
  AND task.status IN ('queued', 'running', 'waiting')
  AND task.kind IN ('file_batch', 'url_batch', 'text_batch')
ORDER BY item.created_at, item.id
