-- Atomic inline success commit for the blocking Docling stage (issue 650 P3).
--
-- Marks one active remote job `succeeded` and persists the fetched sections
-- into its running item's payload in a single statement. Both writes are
-- gated on the worker's item lease (`lease_token` plus `status = 'running'`)
-- via the `owned` guard, so a worker that lost its lease (task cancel, crash
-- reclaim by another worker) changes nothing at all: the row stays active
-- for the rightful owner and the payload is left alone. The remote write
-- only transitions ('pending','running') rows, so a concurrent recovery
-- cancel that already moved the row yields no row either.
WITH owned AS (
    SELECT item.id AS item_id
    FROM context69.task_items AS item
    WHERE item.id = $2
      AND item.lease_token = $5
      AND item.status = 'running'
),
finished AS (
    UPDATE context69.task_docling_remote_jobs AS job
    SET status = 'succeeded',
        remote_status = $4,
        last_polled_at = now(),
        finished_at = now(),
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    WHERE job.id = $1
      AND job.status IN ('pending', 'running')
      AND EXISTS (SELECT 1 FROM owned)
    RETURNING job.id
)
UPDATE context69.task_items AS item
SET payload = $3,
    updated_at = now()
FROM finished
WHERE item.id = $2
  AND item.lease_token = $5
  AND item.status = 'running'
RETURNING item.id
