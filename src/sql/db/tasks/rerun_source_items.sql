-- Source rows for the internal fresh-source rerun fork (issue 1138 P17).
--
-- The source task row is locked first, so the whole fork owns the source task
-- before any payload is read. Only a succeeded parent of a file/url/text batch
-- may be forked, and only its succeeded items that still own a library file are
-- selected.
--
-- The item payload is rebuilt from the canonical submission options only: the
-- parent's saved sections, indexing checkpoint, pipeline progress and download
-- artifact are never copied, so the new item must read the current source
-- again. `input_storage_object_id` is never inherited either.
WITH locked_task AS MATERIALIZED (
    SELECT task.id, task.kind, task.group_id, task.status
    FROM context69.tasks AS task
    WHERE task.id = $1
    FOR UPDATE
)
SELECT item.file_id,
       CASE
           WHEN locked_task.kind IN ('file_batch', 'url_batch')
               AND item.payload ? 'options'
               THEN jsonb_build_object('options', item.payload -> 'options')
           ELSE '{}'::jsonb
       END AS payload
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
JOIN locked_task ON locked_task.id = task.id
JOIN context69.library_files file ON file.id = item.file_id
WHERE item.task_id = $1
  AND item.status = 'succeeded'
  AND item.file_id IS NOT NULL
  AND locked_task.status = 'succeeded'
  AND locked_task.kind IN ('file_batch', 'url_batch', 'text_batch')
  -- The file must still belong to the source task's group, so a stale item can
  -- never fork another group's file into a fresh read.
  AND file.group_id = locked_task.group_id
  AND lower(file.filename) <> 'source.json'
  AND (file.external_id IS NULL OR file.external_id NOT LIKE 'source-folder:%')
  -- Skip files that already have a live item in another task so a source rerun
  -- can never create a second active processing task for one file.
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items active
      WHERE active.file_id = item.file_id
        AND active.id <> item.id
        AND active.status IN ('queued', 'running', 'waiting')
  )
ORDER BY item.ordinal
FOR UPDATE OF item
