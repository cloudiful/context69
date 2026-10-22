DELETE FROM cloudiful_tasks.tasks WHERE payload->>'item_id' = $1
