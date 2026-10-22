-- The parked transition of one task item (issue 1020 P2).
--
-- The guard is the captured execution of the item, like the terminal
-- transition: the fencing token of the library attempt, the library task and
-- the execution generation, compared after the row lock was taken and against
-- the clock of the database.
--
-- `$1` is the item, `$2` the fencing token, `$3` the waiting reason, `$4` the
-- dependency key, `$5` the retry time, `$6` the error message, `$7` the
-- library task and `$8` the execution generation.
WITH waiting AS (
    UPDATE context69.task_items
    SET status = 'waiting',
        waiting_reason = $3,
        dependency_key = $4,
        next_attempt_at = $5,
        lease_token = NULL,
        lease_until = NULL,
        error_message = $6,
        waiting_since = COALESCE(waiting_since, now()),
        updated_at = now()
    WHERE id = $1
      AND lease_token = $2
      AND status = 'running'
      AND lease_until > clock_timestamp()
      AND execution_task_id = $7
      AND execution_generation = $8
    RETURNING id
),
attempt_waited AS (
    UPDATE context69.task_attempts
    SET status = 'waiting',
        error_message = $6,
        finished_at = now()
    WHERE item_id = $1
      AND finished_at IS NULL
      AND EXISTS (SELECT 1 FROM waiting)
    RETURNING id
)
SELECT EXISTS (SELECT 1 FROM waiting) AS "updated!"
