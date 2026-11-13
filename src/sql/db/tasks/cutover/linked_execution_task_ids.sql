-- Every library task linked to a domain item (issue 1020 P4A).
--
-- The handover checks the library half of quiescence through the queue
-- capability, which refuses a missing or foreign identifier. This narrow read
-- supplies the complete linked population, deduplicated and globally sorted, so
-- every identifier is checked in the caller's transaction.
SELECT DISTINCT execution_task_id AS "execution_task_id!"
FROM context69.task_items
WHERE execution_task_id IS NOT NULL
ORDER BY execution_task_id
