UPDATE context69.task_items SET execution_task_id = NULL, execution_generation = 0, lease_token = NULL, lease_until = NULL WHERE id = $1
