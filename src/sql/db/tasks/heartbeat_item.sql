-- The lease mirror of one running task item (issue 1020 P2).
--
-- The deadline is the one the library returned for the renewed attempt, so
-- the domain never extends a lease on its own clock. The guard is the
-- captured execution of the item: the fencing token of the library attempt,
-- the library task and the execution generation.
--
-- `$1` is the item, `$2` the fencing token, `$3` the deadline the library
-- returned, `$4` the library task and `$5` the execution generation.
UPDATE context69.task_items
SET lease_until = $3,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
  AND execution_task_id = $4
  AND execution_generation = $5
