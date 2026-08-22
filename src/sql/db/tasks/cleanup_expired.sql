DELETE FROM context69.tasks
WHERE id IN (
    SELECT id
    FROM context69.tasks candidate
    WHERE candidate.status IN ('succeeded', 'failed', 'cancelled')
      AND COALESCE(candidate.finished_at, candidate.updated_at) < $1
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items item
          JOIN context69.task_external_jobs job ON job.item_id = item.id
          WHERE item.task_id = candidate.id
            AND job.status IN ('submitting', 'pending', 'running')
      )
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
RETURNING id
