INSERT INTO context69.task_attempts (task_id, item_id, attempt, status)
         VALUES ($1, $2, 1, 'running') RETURNING id
