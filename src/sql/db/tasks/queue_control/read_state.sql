-- The durable global admission state of the task queue.
--
-- The singleton row always exists, so a deployment that never paused reads as
-- unpaused. A claim reads it inside its own transaction, after the admission
-- lock, so a committed pause is always observed before any item is selected.
SELECT paused, updated_at
FROM context69.task_queue_control
WHERE id = 1
