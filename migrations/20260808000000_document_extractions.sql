CREATE TABLE context69.extraction_templates (
    template_key TEXT PRIMARY KEY,
    version INTEGER NOT NULL DEFAULT 1,
    description TEXT,
    system_prompt TEXT NOT NULL,
    output_schema JSONB NOT NULL,
    max_output_tokens INTEGER NOT NULL DEFAULT 8192,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_extraction_template_max_tokens CHECK (max_output_tokens > 0),
    CONSTRAINT chk_extraction_template_schema_object CHECK (jsonb_typeof(output_schema) = 'object')
);

CREATE TABLE context69.document_extraction_jobs (
    id UUID PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    template_key TEXT NOT NULL REFERENCES context69.extraction_templates(template_key) ON DELETE RESTRICT,
    template_version INTEGER NOT NULL,
    source_record_hash TEXT NOT NULL,
    parameters JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'queued',
    provider_key TEXT,
    provider_config_hash TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_document_extraction_job_status CHECK (
        status IN ('queued', 'running', 'succeeded', 'failed', 'skipped')
    )
);

CREATE UNIQUE INDEX uq_document_extraction_jobs_active
    ON context69.document_extraction_jobs (document_id, template_key, source_record_hash)
    WHERE status IN ('queued', 'running');

CREATE INDEX idx_document_extraction_jobs_pending
    ON context69.document_extraction_jobs (status, created_at, id)
    WHERE status IN ('queued', 'running');

CREATE TABLE context69.document_extraction_attempts (
    id BIGSERIAL PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES context69.document_extraction_jobs(id) ON DELETE CASCADE,
    provider_key TEXT NOT NULL,
    provider_config_hash TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    status TEXT NOT NULL,
    latency_ms BIGINT NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_extraction_attempt_status CHECK (
        status IN ('succeeded', 'failed', 'quota_exceeded')
    )
);

CREATE TABLE context69.document_extraction_versions (
    id UUID PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    template_key TEXT NOT NULL REFERENCES context69.extraction_templates(template_key) ON DELETE RESTRICT,
    template_version INTEGER NOT NULL,
    source_record_hash TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    provider_config_hash TEXT NOT NULL,
    model_name TEXT,
    result_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, template_key, template_version, source_record_hash)
);

ALTER TABLE context69.library_files
    ADD COLUMN extraction_template_key TEXT,
    ADD COLUMN extraction_parameters JSONB NOT NULL DEFAULT '{}'::jsonb;
