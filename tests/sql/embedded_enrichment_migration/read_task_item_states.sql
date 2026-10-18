SELECT id,
       failure_stage,
       status,
       error_message,
       attempt_count
FROM context69.task_items
WHERE id = ANY($1)
