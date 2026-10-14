-- Enqueues durable cleanup for every still-active remote job of one task.
--
-- Called inside the same transaction that flips the task ledger to
-- `cancelled` (user cancel, bulk cancel, purge capture): only
-- ('pending','running') rows are captured, each with its immutable
-- submit-time endpoint snapshot and reason. Idempotent via the cleanup
-- uniqueness (endpoint+remote id, legacy job id): a repeated enqueue for the
-- same terminalization inserts zero new rows. No network happens here.
INSERT INTO context69.task_docling_remote_cleanup (
    remote_job_id,
    task_id,
    item_id,
    remote_task_id,
    endpoint_base_url,
    reason
)
SELECT
    job.id,
    job.task_id,
    job.item_id,
    job.remote_task_id,
    job.endpoint_base_url,
    $2
FROM context69.task_docling_remote_jobs AS job
WHERE job.task_id = $1
  AND job.status IN ('pending', 'running')
  AND EXISTS (
      SELECT 1 FROM context69.tasks task
      WHERE task.id = job.task_id
        AND task.status IN ('succeeded', 'failed', 'cancelled')
  )
ON CONFLICT DO NOTHING
RETURNING id;
