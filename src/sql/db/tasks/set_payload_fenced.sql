-- The payload of one running task item, guarded by the captured grant (issue
-- 1020 P3B).
--
-- The guard is the whole grant, not only its token: the fencing token of the
-- library attempt, the running status, the deadline of the lease, the library
-- task and the execution generation. A checkpoint of a superseded generation
-- never overwrites the checkpoint of the generation that replaced it, and the
-- deadline is compared against the clock of the database after the row lock.
--
-- `$1` is the item, `$2` the fencing token, `$3` the library task, `$4` the
-- execution generation and `$5` the payload.
UPDATE context69.task_items
SET payload = $5,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
  AND execution_task_id = $3
  AND execution_generation = $4
