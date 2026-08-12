UPDATE context69.task_items AS item
SET payload = item.payload - 'content_base64',
    updated_at = now()
FROM context69.tasks AS task
WHERE task.id = item.task_id
  AND task.kind = 'file_batch'
  AND item.file_id IS NOT NULL
  AND item.payload ? 'content_base64';
