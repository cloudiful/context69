UPDATE context69.task_items SET status = $2, waiting_reason = NULL, next_attempt_at = now() - interval '1 second' WHERE id = $1
