SELECT count(*) AS "count!" FROM cloudiful_tasks.tasks WHERE payload->>'item_id' = $1
