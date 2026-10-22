-- The domain projection of one library claim (issue 1020 P2).
--
-- The lease of the item is the fencing token of the library attempt and its
-- deadline is the deadline the library returned, so the domain never mints an
-- authority of its own and never extends a lease on its own clock. The domain
-- attempt records both library identities, so the domain history and the
-- library history can be correlated without a lookup of the latest owner.
--
-- `$1` is the item, `$2` the fencing token of the library attempt, `$3` the
-- lease deadline the library returned, `$4` the library task, `$5` the
-- execution generation, `$6` the library attempt, `$7` says that the parent
-- takes a new global slot, and `$8` is the deadline of the slot the parent
-- already holds.
WITH claimed AS (
    UPDATE context69.task_items AS item
    SET status = 'running',
        attempt_count = item.attempt_count + 1,
        lease_token = $2,
        lease_until = $3,
        execution_task_id = $4,
        execution_generation = $5,
        started_at = COALESCE(item.started_at, now()),
        finished_at = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    WHERE item.id = $1
      AND item.status IN ('queued', 'waiting')
    RETURNING item.id,
              item.task_id,
              item.ordinal,
              item.attempt_count,
              item.payload,
              item.file_id,
              item.stage,
              item.input_storage_object_id
),
attempts AS (
    INSERT INTO context69.task_attempts
        (task_id, item_id, attempt, status, execution_task_id, execution_attempt_id)
    SELECT task_id, id, attempt_count, 'running', $4, $6
    FROM claimed
    RETURNING item_id, id AS attempt_id
),
parent AS (
    -- The single parent UPDATE: the global slot of the parent, granted when it
    -- has none and renewed when the one it holds is close to expiring. A
    -- still-fresh slot keeps its token and deadline.
    UPDATE context69.tasks task
    SET lease_token = CASE
            WHEN $7 THEN gen_random_uuid()
            ELSE task.lease_token
        END,
        lease_until = CASE
            WHEN $7
                 OR $8 < now() + interval '4 minutes'
                THEN now() + interval '8 minutes'
            ELSE task.lease_until
        END,
        status = 'running',
        started_at = COALESCE(task.started_at, now()),
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        updated_at = now()
    WHERE task.id = (SELECT task_id FROM claimed)
      AND task.status IN ('queued', 'running', 'waiting')
    RETURNING task.id
)
SELECT claimed.id,
       claimed.task_id,
       claimed.ordinal,
       claimed.attempt_count,
       claimed.payload,
       claimed.file_id,
       claimed.stage,
       claimed.input_storage_object_id,
       attempts.attempt_id
FROM claimed
JOIN attempts ON attempts.item_id = claimed.id
