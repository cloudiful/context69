-- Pause projection of one task item and its parent.
-- $1 is the task item id. Reads the counters and the parent gate without
-- exposing a lease token or an attempt fence to the test.
SELECT task.pause_state AS "pause_state!",
       task.paused_at,
       task.lease_token,
       task.lease_until,
       item.status AS "item_status!",
       item.attempt_count,
       item.paused_attempts,
       item.next_attempt_at,
       item.execution_task_id,
       item.waiting_reason,
       item.dependency_key
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.id = $1
