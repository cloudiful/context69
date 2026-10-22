UPDATE context69.tasks SET status = 'waiting', next_attempt_at = now() - interval '1 minute' WHERE id = $1
