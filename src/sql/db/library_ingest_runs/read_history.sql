SELECT item_id, task_id, file_id, batch_id, generation, mode, source,
       lease_token, execution_task_id, execution_generation, resolved_at, retired_at
FROM context69.library_ingest_runs
WHERE item_id = $1
ORDER BY generation
