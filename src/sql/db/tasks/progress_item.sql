-- The progress transition of one task item (issue 1020 P2).
--
-- The guard is the captured execution of the item: the fencing token of the
-- library attempt, the library task and the execution generation, compared
-- after the row lock was taken and against the clock of the database.
--
-- `$1` is the item, `$2` the fencing token, `$3` the domain attempt, `$4` the
-- library task and `$5` the execution generation.
WITH progressed AS (
    UPDATE context69.task_items
    SET status = 'queued',
        attempt_count = 0,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = now(),
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    WHERE id = $1
      AND lease_token = $2
      AND status = 'running'
      AND lease_until > clock_timestamp()
      AND execution_task_id = $4
      AND execution_generation = $5
    RETURNING id
),
attempt_progressed AS (
    UPDATE context69.task_attempts
    SET status = 'progressed',
        finished_at = now()
    WHERE id = $3
      AND item_id = $1
      AND finished_at IS NULL
      AND EXISTS (SELECT 1 FROM progressed)
    RETURNING id
)
SELECT EXISTS (SELECT 1 FROM progressed) AS "updated!"
