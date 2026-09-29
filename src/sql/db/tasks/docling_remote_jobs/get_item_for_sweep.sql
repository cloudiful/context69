-- Loads one parked item for the Docling sweep finalize path.
--
-- Returns the payload the sweep must patch with the fetched sections, plus
-- the file and status guards so a concurrently cancelled/retried item is
-- detected before any remote finish is committed.
SELECT
    id,
    task_id,
    payload,
    file_id,
    status,
    waiting_reason
FROM context69.task_items
WHERE id = $1
LIMIT 1
