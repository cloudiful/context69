UPDATE context69.library_ingest_runs
SET resolved_at = now(), updated_at = now()
WHERE item_id = $1 AND lease_token = $2 AND execution_task_id = $3
  AND execution_generation = $4 AND resolved_at IS NULL AND retired_at IS NULL
RETURNING item_id
