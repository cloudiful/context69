SELECT CASE
           WHEN task.kind = 'translation' THEN item.payload - 'job_ids'
           ELSE item.payload
       END AS payload,
       item.stage,
       item.file_id,
       item.input_storage_object_id
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.task_id = $1
  AND item.status <> 'succeeded'
ORDER BY item.ordinal
