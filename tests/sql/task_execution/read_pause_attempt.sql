-- Latest attempt of one task item, sealed or open.
-- $1 is the task item id. The pause projection asserts the attempt outcome
-- without reading the lease token or the captured library fence.
SELECT attempt.status AS "status!",
       attempt.failure_stage,
       attempt.error_message,
       attempt.finished_at
FROM context69.task_attempts attempt
WHERE attempt.item_id = $1
ORDER BY attempt.started_at DESC
LIMIT 1
