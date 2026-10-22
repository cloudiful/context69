UPDATE cloudiful_tasks.attempts SET lease_expires_at = now() - interval '1 minute' WHERE task_id = $1 AND fencing_token = $2
