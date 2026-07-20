ALTER TABLE context69.library_ingest_jobs
    ADD COLUMN IF NOT EXISTS failure_stage TEXT;

ALTER TABLE context69.library_url_import_jobs
    ADD COLUMN IF NOT EXISTS failure_stage TEXT;

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_failure_stage
    ON context69.library_ingest_jobs (failure_stage, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_url_import_jobs_failure_stage
    ON context69.library_url_import_jobs (failure_stage, updated_at DESC);
