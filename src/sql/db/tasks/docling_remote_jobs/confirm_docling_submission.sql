-- Confirm one accepted Docling submission (issue 1172 P5A).
--
-- A parsed remote id turns the durable intent into the known-id `pending`
-- state the P4 poll path owns. The write is fenced on the captured execution
-- and the `submitting` status, so a lost lease or an already uncertain row
-- changes nothing and the caller must reread the durable row instead.
--
-- `$1` job, `$2` item, `$3` domain task, `$4` fencing token, `$5` library
-- task, `$6` execution generation, `$7` remote task id and `$8` next poll.
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
SET status = 'pending',
    remote_task_id = $7,
    remote_status = 'submitted',
    next_poll_at = $8,
    last_polled_at = NULL,
    updated_at = clock_timestamp()
WHERE job.id = $1
  AND job.item_id = $2
  AND job.status = 'submitting'
  AND EXISTS (SELECT 1 FROM owned)
RETURNING
    job.id,
    job.task_id,
    job.item_id,
    job.provider,
    job.remote_task_id,
    job.status,
    job.remote_status,
    job.attempt_count,
    job.poll_step,
    job.next_poll_at,
    clock_timestamp() AS "observed_at!",
    job.last_polled_at,
    job.deadline_at,
    job.endpoint_base_url,
    job.lease_token,
    job.lease_until,
    job.last_error,
    job.created_at,
    job.updated_at,
    job.finished_at,
    job.dispatch_attempted,
    job.submission_execution_task_id,
    job.submission_generation,
    job.submission_identity
