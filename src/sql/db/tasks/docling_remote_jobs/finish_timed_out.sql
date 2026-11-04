-- Expire one known Docling remote job whose deadline has passed.
--
-- The item guard is the captured execution, exactly like `record_probe.sql`.
-- The job must still be active and its deadline must have passed on the
-- database clock, so a refused or not-due predicate rolls the caller's library
-- seal back instead of timing out early. The same statement records exactly one
-- durable cleanup row with the real remote identity and the `deadline` reason;
-- the existing unique index makes a repeat insert idempotent.
--
-- `$1` is the job, `$2` the item, `$3` the task, `$4` the fencing token,
-- `$5` the library task, `$6` the execution generation and `$7` the error.
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
),
timed_out AS (
    UPDATE context69.task_docling_remote_jobs AS job
    SET status = 'timed_out',
        remote_status = COALESCE(job.remote_status, job.status),
        last_error = COALESCE($7, job.last_error, 'remote job deadline exceeded'),
        last_polled_at = clock_timestamp(),
        finished_at = clock_timestamp(),
        lease_token = NULL,
        lease_until = NULL,
        updated_at = clock_timestamp()
    WHERE job.id = $1
      AND job.item_id = $2
      AND job.task_id = $3
      AND job.status IN ('pending', 'running')
      AND job.deadline_at IS NOT NULL
      AND job.deadline_at <= clock_timestamp()
      AND EXISTS (SELECT 1 FROM owned)
    RETURNING
        job.id,
        job.task_id,
        job.item_id,
        job.remote_task_id,
        job.endpoint_base_url
), cleanup AS (
    INSERT INTO context69.task_docling_remote_cleanup (
        remote_job_id,
        task_id,
        item_id,
        remote_task_id,
        endpoint_base_url,
        reason
    )
    SELECT id, task_id, item_id, remote_task_id, endpoint_base_url, 'deadline'
    FROM timed_out
    ON CONFLICT DO NOTHING
    RETURNING id
)
SELECT EXISTS (SELECT 1 FROM timed_out) AS "timed_out!"
