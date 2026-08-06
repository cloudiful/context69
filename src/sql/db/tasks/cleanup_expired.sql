DELETE FROM context69.tasks
WHERE id IN (
    SELECT id
    FROM context69.tasks
    WHERE status IN ('succeeded', 'failed', 'cancelled')
      AND COALESCE(finished_at, updated_at) < $1
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
RETURNING id
