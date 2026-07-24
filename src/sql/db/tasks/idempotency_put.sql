INSERT INTO context69.task_idempotency_keys (user_id, idempotency_key, request_hash, task_id)
VALUES ($1, $2, $3, $4)
ON CONFLICT (user_id, idempotency_key) DO NOTHING
