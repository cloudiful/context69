ALTER TABLE context69.runtime_file_library_settings
    ADD COLUMN url_import_concurrency BIGINT NOT NULL DEFAULT 2,
    ADD COLUMN url_import_min_interval_ms BIGINT NOT NULL DEFAULT 1000;

ALTER TABLE context69.library_url_import_jobs
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_expires_at TIMESTAMPTZ;

ALTER TABLE context69.runtime_file_library_settings
    ADD CONSTRAINT chk_runtime_file_library_url_import_concurrency
        CHECK (url_import_concurrency > 0),
    ADD CONSTRAINT chk_runtime_file_library_url_import_min_interval_ms
        CHECK (url_import_min_interval_ms > 0);

-- The previous process model had no owner token. Deployment must stop old
-- workers before applying this migration so active rows can be safely queued.
UPDATE context69.library_url_import_jobs
SET status = 'queued',
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE status IN ('downloading', 'ingesting');

CREATE INDEX idx_library_url_import_jobs_queue_claim
    ON context69.library_url_import_jobs (created_at, id)
    WHERE status = 'queued';
