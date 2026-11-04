-- Latest Docling submission history row per item of a bounded task set
-- (issue 1172 P5B1).
--
-- Read-only diagnose projection: one row per item, never the frozen identity,
-- endpoint, remote id, last_error or response body. A current unresolved row
-- wins over a superseded one; equal timestamps fall back to the newest id so
-- the choice is deterministic. `$1` is the task-id array.
SELECT DISTINCT ON (job.item_id)
    job.item_id,
    job.status,
    job.dispatch_attempted,
    job.created_at,
    job.finished_at,
    item.status AS item_status,
    item.waiting_reason AS item_waiting_reason,
    item.lease_token AS item_lease_token,
    item.lease_until AS item_lease_until
FROM context69.task_docling_remote_jobs AS job
JOIN context69.task_items AS item
  ON item.id = job.item_id
 AND item.task_id = job.task_id
WHERE job.task_id = ANY($1::uuid[])
  AND job.status IN ('submitting', 'submission_uncertain', 'superseded')
ORDER BY
    job.item_id,
    (job.status IN ('submitting', 'submission_uncertain')) DESC,
    job.updated_at DESC,
    job.id DESC
