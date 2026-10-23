-- The waiting item with the schedule it waits for.
SELECT status, waiting_reason, next_attempt_at
FROM context69.task_items
WHERE id = $1
