-- The failed item with the failure it records.
SELECT status, failure_stage, error_message, retryable, payload
FROM context69.task_items
WHERE id = $1
