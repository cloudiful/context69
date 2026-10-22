SELECT execution_task_id, execution_attempt_id
           FROM context69.task_attempts WHERE id = $1
