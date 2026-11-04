-- Exhaust the internal execution-failure budget of one item.
--
-- The claim ordinal stays untouched: it is monotonic audit history, not the
-- exhaustion input.
UPDATE context69.task_items SET execution_failure_count = 5 WHERE id = $1
