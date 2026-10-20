-- Source rows for `rerun_task` (issue #975 P4).
--
-- The source task row is locked first and materialized, so the whole rerun
-- transaction owns the source task before any item payload or artifact
-- reference is read. A concurrent task delete (which must take the same row
-- lock) can therefore neither remove a referenced object nor commit between the
-- payload snapshot and the artifact copy.
WITH locked_task AS MATERIALIZED (
    SELECT task.id
    FROM context69.tasks AS task
    WHERE task.id = $1
    FOR UPDATE
)
SELECT item.id,
       CASE
           WHEN task.kind = 'translation' THEN item.payload - 'job_ids'
           ELSE item.payload
       END AS payload,
       item.file_id,
       item.input_storage_object_id
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
JOIN locked_task ON locked_task.id = task.id
WHERE item.task_id = $1
  AND item.status <> 'succeeded'
  -- Skip files that already have a live item in a newer task so a rerun can
  -- never create the second active processing task for one file.
  AND (
      item.file_id IS NULL
      OR NOT EXISTS (
          SELECT 1
          FROM context69.task_items active
          WHERE active.file_id = item.file_id
            AND active.id <> item.id
            AND active.status IN ('queued', 'running', 'waiting')
      )
  )
ORDER BY item.ordinal
FOR UPDATE OF item
