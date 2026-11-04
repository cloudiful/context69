-- The terminal transition of one task item (issue 1020 P2).
--
-- The guard is the captured execution of the item: the fencing token of the
-- library attempt, the library task, and the execution generation. The lease
-- deadline is compared against the clock of the database, and the row lock of
-- the update is taken before the comparison, so a worker that lost its lease
-- while it waited for the lock cannot write.
--
-- `$1` is the item, `$2` the status, `$3` the resource, `$4` the failure
-- stage, `$5` the error message, `$6` the retryable flag, `$7` the fencing
-- token of the library attempt, `$8` the domain attempt, `$9` the library task
-- and `$10` the execution generation.
--
-- `$11` is an explicit next execution-failure budget (used by the dedicated
-- failure transition), `$12` an optional expected budget guard and `$13` an
-- optional parent fence. Without `$11`, a plain `failed` finish increments the
-- budget exactly once (saturating at five); success/cancelled preserve it.
WITH finished AS (
    UPDATE context69.task_items
    SET status = $2,
        execution_failure_count = CASE
            WHEN $11::integer IS NOT NULL
                THEN LEAST(GREATEST($11::integer, 0), 5)
            WHEN $2 = 'failed'
                THEN LEAST(execution_failure_count + 1, 5)
            ELSE execution_failure_count
        END,
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
    WHERE id = $1
      AND lease_token = $7
      AND status = 'running'
      AND lease_until > clock_timestamp()
      AND execution_task_id = $9
      AND execution_generation = $10
      AND ($12::integer IS NULL OR execution_failure_count = $12::integer)
      AND ($13::uuid IS NULL OR task_id = $13::uuid)
    RETURNING id
),
attempt_finished AS (
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
