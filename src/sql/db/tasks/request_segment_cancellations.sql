-- The distinct library segments of the items one cancellation covers
-- (issue 1020 P4B).
--
-- `$1` is the parent task, or NULL when the cancellation covers every
-- active task. The domain cancellation and the cancellation of the
-- segments of these items commit in one transaction.
SELECT DISTINCT execution_task_id FROM context69.task_items WHERE execution_task_id IS NOT NULL AND ($1::uuid IS NULL OR task_id = $1)
