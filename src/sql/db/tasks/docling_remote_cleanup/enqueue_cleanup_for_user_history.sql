-- Enqueues durable cleanup for still-active remotes of tasks about to be
-- bulk-cleared by clear_user_task_history ($1 user_id, $2 view).
--
-- Matches exactly the same task scope as clear_user_task_history.sql:
-- completed (untrashed succeeded) or trash (trashed terminal). Only
-- ('pending','running') job rows are captured. Idempotent via cleanup
-- uniqueness. Runs inside the clear transaction before the cascading delete.
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
    'purge'
FROM context69.task_docling_remote_jobs AS job
JOIN context69.tasks AS task ON task.id = job.task_id
WHERE job.status IN ('pending', 'running')
  AND task.user_id = $1
  AND (
    ($2::text = 'completed'
      AND task.deleted_at IS NULL
      AND task.status = 'succeeded')
    OR ($2::text = 'trash'
      AND task.deleted_at IS NOT NULL
      AND task.status IN ('succeeded', 'failed', 'cancelled'))
  )
ON CONFLICT DO NOTHING
RETURNING id;
