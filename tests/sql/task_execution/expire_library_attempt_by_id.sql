UPDATE cloudiful_tasks.attempts SET lease_expires_at = now() - interval '1 minute' WHERE id = $1
