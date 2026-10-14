-- Direct enqueue for a submitted-but-unpersisted orphan remote.
--
-- Used when a submit succeeded remotely but the job row could not be
-- persisted (unique-violation loser): the orphan will never be polled, so it
-- is tracked here by its immutable identity for termination. Idempotent via
-- endpoint+remote uniqueness. $1 remote_job_id (synthetic when no job row),
-- $2 task_id, $3 item_id, $4 remote_task_id, $5 endpoint, $6 reason.
INSERT INTO context69.task_docling_remote_cleanup (
    remote_job_id,
    task_id,
    item_id,
    remote_task_id,
    endpoint_base_url,
    reason
)
SELECT $1, $2, $3, $4, $5, $6
WHERE NOT EXISTS (
    SELECT 1 FROM context69.task_docling_remote_jobs job
    WHERE job.remote_task_id = $4
      AND job.endpoint_base_url IS NOT DISTINCT FROM $5
)
ON CONFLICT DO NOTHING
RETURNING id;
