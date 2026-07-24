SELECT task_id, request_hash
FROM context69.task_idempotency_keys
WHERE user_id = $1 AND idempotency_key = $2
