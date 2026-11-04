SELECT
    job.status AS job_status,
    job.remote_status,
    job.poll_step,
    job.attempt_count AS error_count,
    job.last_error,
    job.next_poll_at,
    job.deadline_at,
    job.finished_at,
    item.status AS item_status,
    item.waiting_reason,
    item.execution_failure_count,
    item.retryable,
    item.lease_token,
    item.next_attempt_at AS item_next_attempt_at,
    item.attempt_count AS claim_ordinal,
    (SELECT count(*)::bigint FROM context69.task_docling_remote_cleanup AS cleanup
     WHERE cleanup.remote_job_id = job.id) AS "cleanup_count!",
    (SELECT max(reason) FROM context69.task_docling_remote_cleanup AS cleanup
     WHERE cleanup.remote_job_id = job.id) AS cleanup_reason
FROM context69.task_docling_remote_jobs AS job
JOIN context69.task_items AS item ON item.id = job.item_id
WHERE job.id = $1
