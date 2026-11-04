-- The bounded eligible backlog the explicit admission reclassification may
-- convert (issue 1172 P6A2).
--
-- The item must be a queued or waiting non-leased head of an active parent,
-- linked to the current queue's QUEUED segment whose class is still NULL. A
-- Docling submitting/uncertain row is reported so the caller can count it and
-- leave it inert instead of cancelling or reclassifying an unknown outcome.
SELECT item.id,
       item.task_id,
       task.kind,
       item.execution_task_id,
       item.execution_generation,
       EXISTS (
           SELECT 1
           FROM context69.task_docling_remote_jobs job
           WHERE job.item_id = item.id
             AND job.status IN ('submitting', 'submission_uncertain')
       ) AS "docling_inert!"
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
JOIN cloudiful_tasks.tasks old
  ON old.id = item.execution_task_id
 AND old.namespace = 'context69'
 AND old.queue = 'items'
WHERE item.status IN ('queued', 'waiting')
  AND item.lease_token IS NULL
  AND task.status IN ('queued', 'running', 'waiting')
  AND task.deleted_at IS NULL
  AND old.class IS NULL
  AND old.status = 'queued'
ORDER BY task.created_at, item.id
LIMIT $1
FOR UPDATE OF item
