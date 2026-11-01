-- Simulate the passage of the retry time of one exhausted item.
--
-- The item and its parent task carry the same retry time: the parent copy is
-- what the maintenance reads to decide whether the task is due, so the
-- fixture moves both. Without that the case would depend on the clock of the
-- application host and the clock of the database being aligned, which is no
-- property of the exhaustion at all.
WITH exhausted AS (
    UPDATE context69.task_items
    SET attempt_count = 5,
        next_attempt_at = now() - interval '1 second'
    WHERE id = $1
    RETURNING task_id
)
UPDATE context69.tasks
SET next_attempt_at = now() - interval '1 second'
WHERE id IN (SELECT task_id FROM exhausted)
