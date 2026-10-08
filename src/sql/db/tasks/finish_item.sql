-- Finish one item and close its attempt as one unit.
--
-- The statement reports `updated` from the item transition, never from the
-- `task_attempts` row count. An attempt is append-only forensics: maintenance
-- may have interrupted it, or the worker may hold a stale attempt id, and
-- neither changes the fact that the item reached a terminal status. Deciding
-- the parent recompute from the attempt rows instead made a successful finish
-- look like a lost lease and skipped the parent projection.
WITH finished AS (
    UPDATE context69.task_items
    SET status = $2,
        -- The collapsed worker finishes an item at the end of its inline
        -- pipeline, so a succeeded item lands on the terminal `finalize` stage.
        -- A failed item keeps the `processing` stage and reports the precise
        -- `failure_stage` instead.
        stage = CASE WHEN $2 = 'succeeded' THEN 'finalize' ELSE stage END,
        resource_id = $3,
        failure_stage = $4,
        error_message = $5,
        retryable = $6,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = now(),
        updated_at = now()
    WHERE id = $1 AND lease_token = $7 AND status = 'running'
    RETURNING id
), attempt_finished AS (
    UPDATE context69.task_attempts
    SET status = $2,
        retryable = $6,
        failure_stage = $4,
        error_message = $5,
        finished_at = now()
    WHERE id = $8
      AND item_id = $1
      AND finished_at IS NULL
      AND EXISTS (SELECT 1 FROM finished)
    RETURNING id
)
SELECT EXISTS (SELECT 1 FROM finished) AS "updated!"
