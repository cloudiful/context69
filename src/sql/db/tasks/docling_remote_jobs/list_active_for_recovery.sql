-- Recovery listing for the blocking Docling flow (issue 650 P3).
--
-- Returns every non-terminal remote job with its owning item/task state so
-- the recovery pass can fence orphaned rows (cancel) and adopt pre-P3 parked
-- items (requeue) without polling any remote endpoint. `adopt_payload`
-- carries the item payload only for legacy `waiting/docling` parks, the one
-- case that requeues with unchanged content; every other row returns NULL so
-- the periodic pass never drags conversion payloads through the listing.
SELECT
    job.id,
    job.task_id,
    job.item_id,
    job.remote_task_id,
    job.deadline_at,
    item.status AS item_status,
    item.waiting_reason AS item_waiting_reason,
    item.lease_until AS item_lease_until,
    task.status AS task_status,
    CASE
        WHEN item.status = 'waiting' AND item.waiting_reason = 'docling'
        THEN item.payload
        ELSE NULL
    END AS adopt_payload
FROM context69.task_docling_remote_jobs AS job
JOIN context69.task_items AS item ON item.id = job.item_id
JOIN context69.tasks AS task ON task.id = job.task_id
WHERE job.status IN ('pending', 'running')
ORDER BY job.created_at, job.id
