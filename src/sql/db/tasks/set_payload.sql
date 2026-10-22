-- The payload of one running task item (issue 1020 P2).
--
-- The guard is the lease of the item: the fencing token of the library
-- attempt, the running status and the deadline of the lease, compared against
-- the clock of the database after the row lock was taken. The execution
-- generation is not available at every call site of this primitive, so the
-- lease itself is what fences the write.
--
-- `$1` is the item, `$2` the fencing token and `$3` the payload.
UPDATE context69.task_items
SET payload = $3,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
