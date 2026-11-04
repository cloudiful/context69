SELECT attempt_count, execution_failure_count, dependency_key, next_attempt_at, waiting_reason FROM context69.task_items WHERE id = $1
