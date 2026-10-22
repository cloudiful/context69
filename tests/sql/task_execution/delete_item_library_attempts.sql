DELETE FROM cloudiful_tasks.attempts
WHERE task_id IN (SELECT id FROM cloudiful_tasks.tasks WHERE payload->>'item_id' = $1)
