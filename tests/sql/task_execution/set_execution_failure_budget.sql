UPDATE context69.task_items
SET execution_failure_count = LEAST(GREATEST($2, 0), 5)
WHERE id = $1
