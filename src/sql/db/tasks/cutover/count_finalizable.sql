-- The number of unlinked running items whose lease is gone (issue 1100 P1).
--
-- This is the population the bounded finalize-interrupted batch selects: a
-- running item the library does not execute, whose domain mirror lease is
-- already gone. The count is read before every batch, so the handover stops
-- as soon as the population is exhausted and reports the remainder when it
-- reaches its bound.
SELECT count(*) AS "count!"
FROM context69.task_items item
WHERE item.execution_task_id IS NULL
  AND item.status = 'running'
  AND (item.lease_until IS NULL OR item.lease_until <= clock_timestamp())
