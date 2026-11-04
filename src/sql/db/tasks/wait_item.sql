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
--
-- `$9` is an explicit next execution-failure budget, `$10` an optional expected
-- budget guard and `$11` an optional parent fence. A plain park never keys on a
-- freeform reason string and preserves the budget; only the dedicated failure
-- transition supplies a next value.
WITH waiting AS (
    UPDATE context69.task_items
    SET status = 'waiting',
        execution_failure_count = CASE
            WHEN $9::integer IS NULL THEN execution_failure_count
            ELSE LEAST(GREATEST($9::integer, 0), 5)
        END,
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
      AND ($10::integer IS NULL OR execution_failure_count = $10::integer)
      AND ($11::uuid IS NULL OR task_id = $11::uuid)
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
