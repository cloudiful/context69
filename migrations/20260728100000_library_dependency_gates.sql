CREATE TABLE IF NOT EXISTS context69.library_dependency_gates (
    dependency_key TEXT PRIMARY KEY,
    state TEXT NOT NULL DEFAULT 'closed',
    failure_count INTEGER NOT NULL DEFAULT 0,
    next_probe_at TIMESTAMPTZ,
    last_error TEXT,
    configuration_fingerprint TEXT,
    probe_lease_token UUID,
    probe_lease_expires_at TIMESTAMPTZ,
    last_transition_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_success_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (state IN ('closed', 'open', 'half_open')),
    CHECK (failure_count >= 0)
);

ALTER TABLE context69.library_dependency_gates
    ADD COLUMN IF NOT EXISTS configuration_fingerprint TEXT;

INSERT INTO context69.library_dependency_gates (dependency_key)
VALUES ('s3'), ('docling'), ('embedding_vector')
ON CONFLICT (dependency_key) DO NOTHING;

ALTER TABLE context69.library_ingest_jobs
    ADD COLUMN IF NOT EXISTS lease_token UUID,
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS requires_docling BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS section_payload JSONB;

ALTER TABLE context69.library_url_import_jobs
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

-- The old process model did not own a durable lease. Requeue those rows so a
-- deployment cannot leave them permanently running.
UPDATE context69.library_ingest_jobs
SET status = 'pending',
    lease_token = NULL,
    lease_expires_at = NULL,
    started_at = NULL,
    finished_at = NULL,
    updated_at = now()
WHERE status = 'running';

UPDATE context69.library_url_import_jobs
SET status = CASE
        WHEN status = 'ingesting' AND ingest_job_id IS NOT NULL THEN 'ingesting'
        ELSE 'queued'
    END,
    next_attempt_at = CASE
        WHEN status = 'ingesting' AND ingest_job_id IS NOT NULL THEN NULL
        ELSE now() + INTERVAL '30 seconds'
    END,
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    finished_at = NULL,
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE status = 'ingesting';

UPDATE context69.library_url_import_jobs url_job
SET status = 'queued',
    next_attempt_at = now() + INTERVAL '30 seconds',
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    finished_at = NULL,
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE url_job.status = 'ingesting'
  AND url_job.ingest_job_id IN (
      SELECT id
      FROM context69.library_ingest_jobs
      WHERE status = 'pending'
  );

UPDATE context69.library_url_import_jobs url_job
SET status = 'queued',
    next_attempt_at = now() + INTERVAL '30 seconds',
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    finished_at = NULL,
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE url_job.status = 'ingesting'
  AND NOT EXISTS (
      SELECT 1
      FROM context69.library_ingest_jobs ingest_job
      WHERE ingest_job.id = url_job.ingest_job_id
        AND ingest_job.status IN ('pending', 'running', 'succeeded', 'failed')
  );

-- A URL may have retained its ingesting status after the old process finished
-- the child job. Reconcile those terminal child states during the migration
-- so the URL API does not depend on a later worker pass to become consistent.
UPDATE context69.library_url_import_jobs url_job
SET status = CASE ingest.status
        WHEN 'succeeded' THEN 'succeeded'
        WHEN 'failed' THEN 'failed'
    END,
    error_code = CASE
        WHEN ingest.status = 'failed' THEN COALESCE('ingest_' || ingest.failure_stage, 'ingest_failed')
        ELSE NULL
    END,
    error_message = CASE
        WHEN ingest.status = 'failed' THEN ingest.error_message
        ELSE NULL
    END,
    failure_stage = CASE
        WHEN ingest.status = 'failed' THEN ingest.failure_stage
        ELSE NULL
    END,
    next_attempt_at = NULL,
    finished_at = COALESCE(ingest.finished_at, now()),
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
FROM context69.library_ingest_jobs ingest
WHERE url_job.ingest_job_id = ingest.id
  AND url_job.status = 'ingesting'
  AND ingest.status IN ('succeeded', 'failed');

UPDATE context69.library_files
SET ingest_status = 'pending',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
WHERE ingest_status = 'running';

UPDATE context69.library_ingest_jobs job
SET requires_docling = (
        lower(file.filename) LIKE '%.pdf'
        OR lower(file.filename) LIKE '%.docx'
        OR lower(file.filename) LIKE '%.xlsx'
        OR file.media_type IN (
            'application/pdf',
            'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
            'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
        )
    )
FROM context69.library_files file
WHERE file.id = job.file_id;

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_queue_claim
    ON context69.library_ingest_jobs (created_at, id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_lease
    ON context69.library_ingest_jobs (status, lease_expires_at);

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_file_status
    ON context69.library_ingest_jobs (file_id, status, created_at);

CREATE INDEX IF NOT EXISTS idx_library_url_import_jobs_next_attempt
    ON context69.library_url_import_jobs (status, next_attempt_at, created_at)
    WHERE status = 'queued';

CREATE INDEX IF NOT EXISTS idx_library_dependency_gates_probe
    ON context69.library_dependency_gates (state, next_probe_at);
