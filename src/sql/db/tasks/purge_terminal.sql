DELETE FROM context69.tasks
WHERE id IN (
    SELECT id
    FROM context69.tasks
    WHERE status IN ('succeeded', 'failed', 'cancelled')
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
RETURNING id
