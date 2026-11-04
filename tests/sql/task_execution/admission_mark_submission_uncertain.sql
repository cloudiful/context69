-- Marks one exact fixture-owned unresolved submission unknown
-- (issue 1172 P6A2). Only the job of the given task/item that is still
-- submitting with no remote id is touched; provenance, budget and deadlines
-- stay as they were.
UPDATE context69.task_docling_remote_jobs job
SET status = 'submission_uncertain',
    last_error = $3
FROM context69.tasks task
WHERE job.item_id = $1
  AND job.task_id = $2
  AND task.id = job.task_id
  AND job.status = 'submitting'
  AND job.remote_task_id IS NULL
RETURNING job.id
