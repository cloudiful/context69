INSERT INTO context69.task_items
    (id, task_id, ordinal, payload, status, failure_stage, error_message,
     attempt_count)
VALUES ($1, $2, 0, '{}'::jsonb, 'failed', $3, 'kept error', 2)
