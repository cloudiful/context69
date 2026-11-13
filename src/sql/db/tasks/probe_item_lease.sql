-- The plain ownership probe of one running task item (issue 55 P1).
--
-- The mirror update is skipped when a page transaction holds the item row, and
-- a skipped update is not a lost lease. This read takes no lock, so it reports
-- the last committed grant even while another transaction holds the row: a
-- matching row means the mirror is busy under this owner, and a missing row
-- means the captured execution no longer owns the item.
--
-- `$1` is the item, `$2` the fencing token, `$3` the library task and `$4` the
-- execution generation.
SELECT 1
FROM context69.task_items
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND execution_task_id = $3
  AND execution_generation = $4
