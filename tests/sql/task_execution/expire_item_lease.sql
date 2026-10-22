UPDATE context69.task_items SET lease_until = now() - interval '1 minute' WHERE id = $1
