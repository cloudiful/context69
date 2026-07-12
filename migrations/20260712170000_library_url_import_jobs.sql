CREATE TABLE context69.library_url_import_jobs (
    id UUID PRIMARY KEY,
    group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE,
    visibility TEXT NOT NULL,
    folder_id UUID REFERENCES context69.library_folders(id) ON DELETE SET NULL,
    source_url TEXT NOT NULL,
    dedupe_key TEXT NOT NULL,
    requested_filename TEXT,
    requested_media_type TEXT,
    external_id TEXT,
    source_uri TEXT,
    published_at TIMESTAMPTZ,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata_provided BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'queued',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    file_id UUID REFERENCES context69.library_files(id) ON DELETE SET NULL,
    ingest_job_id UUID REFERENCES context69.library_ingest_jobs(id) ON DELETE SET NULL,
    error_code TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_library_url_import_status CHECK (
        status IN ('queued', 'downloading', 'ingesting', 'succeeded', 'failed')
    ),
    CONSTRAINT chk_library_url_import_metadata_object CHECK (jsonb_typeof(metadata_json) = 'object')
);

CREATE UNIQUE INDEX uq_library_url_import_jobs_active
    ON context69.library_url_import_jobs (group_id, dedupe_key)
    WHERE status IN ('queued', 'downloading', 'ingesting');

CREATE INDEX idx_library_url_import_jobs_pending
    ON context69.library_url_import_jobs (status, created_at)
    WHERE status IN ('queued', 'downloading', 'ingesting');
