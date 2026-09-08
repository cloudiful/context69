-- Raise the Docling remote admission default ceiling from 1 to 2 (issue #209).
--
-- Two local scheduler workers were contending for a single remote slot, so
-- the default now matches `runtime.scheduler.max_concurrency = 2`. Fresh
-- deployments get the new column default; existing rows still pinned at the
-- old default of 1 are backfilled to 2. Rows an operator deliberately set to
-- any other value in 1..32 are left untouched.
ALTER TABLE context69.docling_settings
    ALTER COLUMN max_inflight SET DEFAULT 2;

UPDATE context69.docling_settings SET max_inflight=2 WHERE max_inflight=1;
