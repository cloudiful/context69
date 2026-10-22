UPDATE context69.task_items SET status = 'waiting', next_attempt_at = now() - interval '1 minute', attempt_count = 0 WHERE id = $1
