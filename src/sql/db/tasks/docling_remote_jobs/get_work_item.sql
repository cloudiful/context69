-- Loads one item for the blocking Docling worker path.
--
-- Returns the payload the worker patches with the fetched sections, plus
-- the file and status guards so a concurrently cancelled/retried item is
-- detected before any remote finish is committed.
SELECT
    id,
    task_id,
    payload,
    file_id,
    status,
    waiting_reason,
    lease_token
FROM context69.task_items
WHERE id = $1
LIMIT 1
