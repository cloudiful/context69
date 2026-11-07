-- The lowest non-terminal item of one parent and the parent pause gate.
--
-- The item row is locked with the same ordinal selection the claim path uses,
-- so a pause serializes with the claim that would run that item. The parent
-- row is only read, never locked: the canonical order is item -> epoch ->
-- segment -> task, and the task row is the last mutation of the caller.
WITH lowest AS (
    SELECT min(item.ordinal) AS ordinal
    FROM context69.task_items item
    WHERE item.task_id = $1
      AND item.status IN ('queued', 'running', 'waiting')
),
current_item AS (
    SELECT item.id,
           item.task_id,
           item.status,
           item.attempt_count,
           item.paused_attempts,
           item.execution_task_id,
           item.execution_generation,
           item.dependency_key,
           item.waiting_reason
    FROM context69.task_items item
    JOIN lowest ON lowest.ordinal = item.ordinal
    WHERE item.task_id = $1
      AND item.status IN ('queued', 'running', 'waiting')
    FOR UPDATE OF item
)
SELECT current_item.id AS "item_id!",
       current_item.status AS "item_status!",
       current_item.execution_task_id,
       current_item.dependency_key,
       current_item.waiting_reason,
       task.kind AS "task_kind!",
       task.status AS "task_status!",
       task.pause_state AS "task_pause_state!"
FROM current_item
JOIN context69.tasks task ON task.id = current_item.task_id
