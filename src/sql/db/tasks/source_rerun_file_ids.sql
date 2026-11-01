-- Files a succeeded source task's items reference (issue 1138 P17).
--
-- The source task row is locked here, before the fork reads any payload, so a
-- concurrent delete cannot remove the file rows the fork is about to reuse.
-- Only a succeeded file/url/text parent contributes files; the files are then
-- locked (and revalidated for ownership and release) by
-- `claim_file_processing_slots` before the selection's active-sibling check
-- runs, exactly like the failed retry and terminal rerun paths.
WITH locked_task AS MATERIALIZED (
    SELECT task.id, task.group_id
    FROM context69.tasks AS task
    WHERE task.id = $1
      AND task.status = 'succeeded'
      AND task.kind IN ('file_batch', 'url_batch', 'text_batch')
    FOR UPDATE
)
SELECT DISTINCT item.file_id
FROM context69.task_items item
JOIN locked_task ON locked_task.id = item.task_id
JOIN context69.library_files file ON file.id = item.file_id
WHERE item.status = 'succeeded'
  AND item.file_id IS NOT NULL
  AND lower(file.filename) <> 'source.json'
  AND (file.external_id IS NULL OR file.external_id NOT LIKE 'source-folder:%')
