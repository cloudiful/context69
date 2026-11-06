-- Lock the domain item of one worker before it touches its library segment
-- (issue 1212 R3).
--
-- The canonical order of the domain is item -> epoch -> segment -> task. A
-- worker therefore locks its domain item before it seals, fails, renews or
-- parks the library segment, and a cancellation locks the same item before it
-- requests the cancellation of that segment. The two paths can no longer take
-- each other's locks in the opposite order.
--
-- The row lock is taken before the grant is compared, so a worker that waited
-- for the lock decides on the row as it is after the transition it waited for.
-- The lease deadline is compared against the clock of the database, never
-- against the worker's clock.
--
-- `owned` reports whether the captured grant still describes the item: the
-- fencing token of the library attempt, the library task, the execution
-- generation, the running status and the lease deadline. A worker that does
-- not own the item any more must not seal, fail, renew or park its segment.
--
-- `$1` is the item, `$2` the fencing token of the library attempt, `$3` the
-- library task of the segment and `$4` the execution generation.
SELECT
    item.status,
    COALESCE(
        item.lease_token = $2
        AND item.status = 'running'
        AND item.lease_until IS NOT NULL
        AND item.lease_until > clock_timestamp()
        AND item.execution_task_id = $3
        AND item.execution_generation = $4,
        FALSE
    ) AS "owned!: bool",
    COALESCE(
        item.execution_task_id = $3
        AND item.execution_generation = $4,
        FALSE
    ) AS "matches_execution!: bool"
FROM context69.task_items AS item
WHERE item.id = $1
FOR UPDATE OF item
