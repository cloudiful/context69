-- The nonblocking lease mirror of one running task item (issue 55 P1).
--
-- The authoritative deadline is the one the library returned for the renewed
-- attempt. The mirror only ever moves that deadline forward, and it refreshes
-- an expired mirror as long as the returned deadline is still live and the
-- captured execution still owns the row. The candidate row lock is skipped
-- instead of waited for: a long page transaction that holds the item is not a
-- lost lease.
--
-- `$1` is the item, `$2` the fencing token, `$3` the deadline the library
-- returned, `$4` the library task and `$5` the execution generation.
WITH candidate AS (
    SELECT item.id
    FROM context69.task_items AS item
    WHERE item.id = $1
      AND item.lease_token = $2
      AND item.status = 'running'
      AND item.execution_task_id = $4
      AND item.execution_generation = $5
    FOR UPDATE SKIP LOCKED
)
UPDATE context69.task_items AS item
SET lease_until = GREATEST(COALESCE(item.lease_until, $3), $3),
    updated_at = now()
FROM candidate
WHERE item.id = candidate.id
  AND $3 > clock_timestamp()
