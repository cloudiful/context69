INSERT INTO context69.library_ingest_runs
    (item_id, task_id, file_id, batch_id, generation, mode, source,
     lease_token, execution_task_id, execution_generation)
VALUES ($1, $2, $3, $4, nextval('context69.library_ingest_generation_seq'), $5, $6, $7, $8, $9)
RETURNING item_id, task_id, file_id, batch_id, generation, mode, source,
          lease_token, execution_task_id, execution_generation, resolved_at, retired_at
