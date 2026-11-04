-- The current bounded staging progress of one library ingest run (issue 1172
-- P6B1).
--
-- Only the current, non-retired run of the item is returned. A retired row is
-- history and must never provide a resume cursor to a later execution.
--
-- `$1` is the task item.

SELECT
    batch_id,
    generation,
    staged_batches_written,
    staged_plan_fingerprint
FROM context69.library_ingest_runs
WHERE item_id = $1
  AND retired_at IS NULL
