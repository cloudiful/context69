-- A raw insert of a second current row must be refused by the partial unique
-- index, whatever the generation. The caller supplies a fresh batch and
-- generation.
INSERT INTO context69.library_ingest_runs
    (item_id, task_id, file_id, batch_id, generation, mode, source,
     lease_token, execution_task_id, execution_generation)
VALUES ($1, $2, $3, $4, nextval('context69.library_ingest_generation_seq'), $5, $6, $7, $8, $9)
