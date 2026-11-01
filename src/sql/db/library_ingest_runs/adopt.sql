UPDATE context69.library_ingest_runs
SET lease_token = $2, execution_task_id = $3, execution_generation = $4, updated_at = now()
WHERE item_id = $1 AND resolved_at IS NULL AND retired_at IS NULL
RETURNING item_id, task_id, file_id, batch_id, generation, mode, source,
          lease_token, execution_task_id, execution_generation, resolved_at, retired_at
