UPDATE context69.task_items SET next_attempt_at = now() - interval '1 second' WHERE id = $1
