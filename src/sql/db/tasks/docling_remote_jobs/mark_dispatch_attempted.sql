-- Durably mark that the prepared dispatch is about to be invoked (issue 1172 P5A).
--
-- The flag is diagnostic evidence only: it never authorizes an automatic
-- retry, because a set flag proves the send was invoked and a cleared flag
-- does not prove it was not. The captured-execution guard and the
-- `submitting` status are the only write authority.
--
-- `$1` job, `$2` item, `$3` domain task, `$4` fencing token, `$5` library
-- task and `$6` execution generation.
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
SET dispatch_attempted = true,
    updated_at = clock_timestamp()
WHERE job.id = $1
  AND job.item_id = $2
  AND job.status = 'submitting'
  AND EXISTS (SELECT 1 FROM owned)
RETURNING job.id
