-- Persist one bounded Docling probe observation for the item that owns it.
--
-- The item guard is the captured execution: item, task, running status, the
-- unexpired database-clock lease, the library task and the execution
-- generation. Only an active job of that exact item may be updated, so a
-- worker cannot project a job from another item. No lease is taken or renewed.
--
-- `$1` is the job, `$2` the item, `$3` the task, `$4` the fencing token,
-- `$5` the library task, `$6` the execution generation, `$7` the observed
-- remote status, `$8` the normal poll step, `$9` the consecutive error count,
-- `$10` the last error and `$11` the next due time.
WITH owned AS (
    SELECT item.id
    FROM context69.task_items AS item
    WHERE item.id = $2
      AND item.task_id = $3
      AND item.status = 'running'
      AND item.lease_token = $4
      AND item.lease_until > clock_timestamp()
      AND item.execution_task_id = $5
      AND item.execution_generation = $6
)
UPDATE context69.task_docling_remote_jobs AS job
SET remote_status = $7,
    attempt_count = $9,
    poll_step = $8,
    last_error = $10,
    next_poll_at = $11,
    last_polled_at = clock_timestamp(),
    updated_at = clock_timestamp()
WHERE job.id = $1
  AND job.item_id = $2
  AND job.task_id = $3
  AND job.status IN ('pending', 'running')
  AND EXISTS (SELECT 1 FROM owned)
RETURNING job.id
