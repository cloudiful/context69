WITH eligible AS MATERIALIZED (
    SELECT job.*
    FROM context69.task_docling_remote_jobs AS job
    JOIN context69.task_items AS item ON item.id = job.item_id
    JOIN context69.tasks AS task ON task.id = job.task_id
    WHERE job.id = $1
      AND job.status IN ('pending', 'running')
      AND (item.status IN ('succeeded', 'failed', 'cancelled')
           OR task.status IN ('succeeded', 'failed', 'cancelled'))
    FOR UPDATE OF job, item, task
), enqueued AS (
    INSERT INTO context69.task_docling_remote_cleanup (
        remote_job_id, task_id, item_id, remote_task_id, endpoint_base_url, reason
    )
    SELECT id, task_id, item_id, remote_task_id, endpoint_base_url,
           CASE WHEN deadline_at <= now() THEN 'deadline' ELSE $2 END
    FROM eligible
    ON CONFLICT DO NOTHING
)
UPDATE context69.task_docling_remote_jobs AS job
SET status = 'cancelled', finished_at = now(), updated_at = now(),
    lease_token = NULL, lease_until = NULL
FROM eligible
WHERE job.id = eligible.id
RETURNING job.id;
