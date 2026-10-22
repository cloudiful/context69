WITH owned AS MATERIALIZED (
    SELECT job.*
    FROM context69.task_docling_remote_jobs job
    JOIN context69.task_items item ON item.id = job.item_id
    JOIN context69.tasks task ON task.id = job.task_id
    WHERE job.id = $1 AND item.id = $2
      AND job.status IN ('pending', 'running')
      AND item.status = 'running' AND item.lease_token = $3
      AND item.lease_until > clock_timestamp()
      AND item.execution_task_id = $4
      AND item.execution_generation = $5
      AND task.status IN ('queued', 'running', 'waiting')
    FOR UPDATE OF job, item, task
), enqueued AS (
    INSERT INTO context69.task_docling_remote_cleanup (
        remote_job_id, task_id, item_id, remote_task_id, endpoint_base_url, reason
    )
    SELECT id, task_id, item_id, remote_task_id, endpoint_base_url, 'orphan'
    FROM owned
    ON CONFLICT DO NOTHING
)
UPDATE context69.task_docling_remote_jobs job
SET status = 'cancelled', finished_at = now(), updated_at = now(),
    lease_token = NULL, lease_until = NULL
FROM owned
WHERE job.id = owned.id
RETURNING job.id;
