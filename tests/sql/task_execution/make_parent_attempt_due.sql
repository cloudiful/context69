UPDATE context69.tasks SET next_attempt_at = now() - interval '1 second' WHERE id = $1
