-- Cancel the items of one task and report the items it actually cancelled.
--
-- The item rows are locked in id order before they are updated, so two
-- cancellations that cover the same items take the same item locks in one
-- deterministic order and cannot deadlock with each other. The active-status
-- predicate is kept: an item that an activation already completed keeps its
-- succeeded status.
--
-- The cancelled item ids are returned, because only those items own a segment
-- whose cancellation this task may request.
WITH locked AS (
    SELECT item.id
    FROM context69.task_items AS item
    WHERE item.task_id = $1
      AND item.status IN ('queued', 'running', 'waiting')
    ORDER BY item.id
    FOR UPDATE
), cancelled AS (
    UPDATE context69.task_items AS item
    SET status = 'cancelled',
        lease_token = NULL,
        lease_until = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        finished_at = now(),
        updated_at = now()
    FROM locked
    WHERE item.id = locked.id
    RETURNING item.id
), attempts AS (
    UPDATE context69.task_attempts AS attempt
    SET status = 'cancelled',
        retryable = FALSE,
        finished_at = now()
    WHERE attempt.item_id IN (SELECT id FROM cancelled)
      AND attempt.finished_at IS NULL
    RETURNING attempt.id
)
SELECT id FROM cancelled ORDER BY id
