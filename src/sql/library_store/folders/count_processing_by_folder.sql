SELECT file.folder_id AS "folder_id", COUNT(*)::BIGINT AS "count!"
FROM context69.library_files file
WHERE ($1::BIGINT IS NULL OR file.group_id = $1)
  AND EXISTS (
      SELECT 1
      FROM context69.task_items active
      WHERE active.file_id = file.id
        AND active.status IN ('queued', 'running', 'waiting')
  )
GROUP BY file.folder_id
