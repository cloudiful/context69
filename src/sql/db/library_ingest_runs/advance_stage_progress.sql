-- Advance the bounded staging progress of one library ingest run by exactly
-- one verified batch (issue 1172 P6B1).
--
-- The update is fenced by the captured item grant and the adopted run identity,
-- and refuses a retired or resolved run. The caller passes the exact previous
-- count it observed, so a concurrent or stale advance cannot skip a batch. The
-- plan fingerprint is written on the first advance and must stay equal
-- afterwards.
--
-- `$1` is the item, `$2` the run batch id, `$3` the fencing token, `$4` the
-- library task, `$5` the execution generation, `$6` the plan fingerprint and
-- `$7` the expected previous count.

UPDATE context69.library_ingest_runs
SET staged_batches_written = staged_batches_written + 1,
    staged_plan_fingerprint = COALESCE(staged_plan_fingerprint, $6),
    updated_at = now()
WHERE item_id = $1
  AND batch_id = $2
  AND lease_token = $3
  AND execution_task_id = $4
  AND execution_generation = $5
  AND retired_at IS NULL
  AND resolved_at IS NULL
  AND staged_batches_written = $7
  AND (staged_plan_fingerprint IS NULL OR staged_plan_fingerprint = $6)
RETURNING staged_batches_written
