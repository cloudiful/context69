-- Paged task items for `GET /v1/tasks/{task_id}/items` (issue 413 Phase 1).
--
-- `$4::text` narrows to one item status; NULL lists every status.
-- Fixed active-first ordering (failed, running, queued, waiting,
-- cancelled, succeeded, then ordinal) pins in-flight/failed to the top and
-- sinks succeeded to the bottom. `cursor`/`offset` ($3) is scoped to the
-- current status filter: clients reset to 0 when the filter changes, then
-- follow `next_cursor` until NULL to page through the full filtered set.
SELECT item.id,
       item.task_id,
       item.ordinal,
       item.status,
       item.resource_id,
       item.file_id,
       item.stage,
       item.waiting_reason,
       item.dependency_key,
       item.next_attempt_at,
       item.failure_stage,
       item.error_message,
       item.attempt_count,
       item.retryable,
       item.created_at,
       item.started_at,
       item.finished_at
FROM context69.task_items item
WHERE item.task_id = $1
  AND ($4::text IS NULL OR item.status = $4::text)
ORDER BY
    CASE item.status
        WHEN 'failed' THEN 0
        WHEN 'running' THEN 1
        WHEN 'queued' THEN 2
        WHEN 'waiting' THEN 3
        WHEN 'cancelled' THEN 4
        WHEN 'succeeded' THEN 5
        ELSE 6
    END,
    item.ordinal
LIMIT $2 OFFSET $3
