SELECT status, failure_stage, error_message, lease_token, lease_until, finished_at FROM context69.task_items WHERE id = $1
