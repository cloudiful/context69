-- Cancel exactly the eligible task rows of one bulk cancellation (issue 1212 R2).
--
-- `$1` is the selected active task id list. The rows are locked in id order
-- before they are updated, and the UPDATE keeps the active-status predicate:
-- a task that an activation already completed between the selection and this
-- statement is not cancelled. The statement runs after the item locks and the
-- registry epoch, so it completes the canonical item -> epoch -> task order.
WITH targets AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE task.id = ANY($1::uuid[])
      AND task.status IN ('queued', 'running', 'waiting')
    ORDER BY task.id
    FOR UPDATE
)
UPDATE context69.tasks task
SET status = 'cancelled',
    lease_token = NULL,
    lease_until = NULL,
    next_attempt_at = NULL,
    finished_at = now(),
    updated_at = now()
FROM targets
WHERE task.id = targets.id
RETURNING task.id
