-- Rechecks the captured task grant immediately before the commit.
--
-- A write can queue behind a lock for longer than the remaining lease, so the
-- grant is verified again after the business statements ran: the clock is read
-- afresh, and a lease that ran out while the write waited rolls the whole
-- transaction back. The row is already locked by the caller, so no new lock is
-- taken here.
--
-- `$1` is the item, `$2` the fencing token of the library attempt, `$3` the
-- library task of the segment and `$4` the execution generation.
SELECT item.id
FROM context69.task_items item
WHERE item.id = $1
  AND item.lease_token = $2
  AND item.status = 'running'
  AND item.lease_until > clock_timestamp()
  AND item.execution_task_id = $3
  AND item.execution_generation = $4
