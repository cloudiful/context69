-- Full foreign-key lookup coverage for permanent task deletion.
--
-- Task deletion cascades into these child tables; the existing task_attempts
-- and task_docling_remote_jobs indexes are partial, so full btree indexes on
-- every referencing column keep each cascade from scanning the child set.
-- idx_library_ingest_runs_item_history duplicates the library_ingest_runs
-- primary key (item_id, generation) and is dropped; no other index changes.
-- These non-concurrent CREATE/DROP INDEX statements run inside the migration
-- transaction, block writes on the affected tables and hold locks until commit,
-- so apply them in a startup or maintenance window.

CREATE INDEX IF NOT EXISTS idx_task_attempts_item_id
    ON context69.task_attempts (item_id);

CREATE INDEX IF NOT EXISTS idx_task_attempts_task_id
    ON context69.task_attempts (task_id);

CREATE INDEX IF NOT EXISTS idx_task_idempotency_keys_task_id
    ON context69.task_idempotency_keys (task_id);

CREATE INDEX IF NOT EXISTS idx_task_docling_remote_jobs_item_id
    ON context69.task_docling_remote_jobs (item_id);

CREATE INDEX IF NOT EXISTS idx_library_ingest_runs_task_id
    ON context69.library_ingest_runs (task_id);

DROP INDEX IF EXISTS context69.idx_library_ingest_runs_item_history;
