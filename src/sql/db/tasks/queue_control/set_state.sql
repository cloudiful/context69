-- Writes the durable global admission state of the task queue.
--
-- The caller takes the task-admission lock in the same transaction before this
-- statement, so the write serializes against every claim: no claim can admit
-- work after a pause commits. `updated_at` also advances on an idempotent
-- write, so it reports the last committed change of the state.
UPDATE context69.task_queue_control
SET paused = $1,
    updated_at = now()
WHERE id = 1
RETURNING paused, updated_at
