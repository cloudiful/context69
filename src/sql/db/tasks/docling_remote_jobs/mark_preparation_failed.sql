-- Fail one not-dispatched Docling intent during preparation (issue 1172 P5A).
--
-- Preparation runs before `.send()` is invoked, so a preparation failure is
-- definitively pre-dispatch and may never leave an open intent behind. The
-- row is closed as `failed` only while `dispatch_attempted` is still false and
-- the captured execution still holds the item.
--
-- `$1` job, `$2` item, `$3` domain task, `$4` fencing token, `$5` library
-- task, `$6` execution generation and `$7` sanitized reason.
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
SET status = 'failed',
    remote_status = COALESCE(job.remote_status, job.status),
    last_error = COALESCE($7, job.last_error, 'docling preparation failed before dispatch'),
    finished_at = clock_timestamp(),
    lease_token = NULL,
    lease_until = NULL,
    updated_at = clock_timestamp()
WHERE job.id = $1
  AND job.item_id = $2
  AND job.status = 'submitting'
  AND job.dispatch_attempted = false
  AND EXISTS (SELECT 1 FROM owned)
RETURNING job.id
