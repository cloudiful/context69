UPDATE cloudiful_tasks.attempts SET lease_expires_at = now() - interval '1 minute' WHERE status = 'running' AND task_id IN (SELECT execution_task_id FROM context69.task_items WHERE id = $1)
