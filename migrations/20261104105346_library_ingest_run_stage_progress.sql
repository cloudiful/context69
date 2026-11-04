-- Durable bounded staging progress of one library ingest run (issue 1172
-- P6B1).
--
-- The count is the monotonic number of whole embedding batches whose staged
-- points were written and verified by this run. It is a resume cursor, never a
-- work quantum: a task turn may commit one batch and yield with the count
-- persisted. The fingerprint binds the exact prepared staging plan so a
-- changed source, chunking or metadata refuses to skip completed work.
--
-- Old rows start at zero with no fingerprint: their safe replay is from the
-- first batch. Nothing is inferred from the registry staged ids, which describe
-- the total plan rather than completion.

ALTER TABLE context69.library_ingest_runs
    ADD COLUMN staged_batches_written BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN staged_plan_fingerprint TEXT;

ALTER TABLE context69.library_ingest_runs
    ADD CONSTRAINT library_ingest_runs_staged_batches_written_nonnegative
    CHECK (staged_batches_written >= 0);
