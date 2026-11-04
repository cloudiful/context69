-- Relinks one owned queued/waiting item to a fixture-created unclassified
-- library segment (issue 1172 P6A2 test fixture). Domain-only and guarded by
-- the exact old pointer and generation; the library segment itself is created
-- through the public queue API, never by a direct library metadata write.
UPDATE context69.task_items
SET execution_task_id = $2,
    execution_generation = $3,
    updated_at = now()
WHERE id = $1
  AND execution_task_id = $4
  AND execution_generation = $5
  AND status IN ('queued', 'waiting')
RETURNING id
