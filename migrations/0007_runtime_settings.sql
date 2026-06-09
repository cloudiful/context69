CREATE TABLE IF NOT EXISTS context69.runtime_provider_accounts (
    account_key TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key TEXT,
    disabled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(account_key) <> ''),
    CHECK (btrim(provider_kind) <> ''),
    CHECK (btrim(display_name) <> ''),
    CHECK (btrim(base_url) <> '')
);

CREATE TABLE IF NOT EXISTS context69.runtime_qdrant_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    url TEXT NOT NULL,
    collection_name TEXT NOT NULL,
    recreate_on_dimension_mismatch BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (btrim(url) <> ''),
    CHECK (btrim(collection_name) <> '')
);

CREATE TABLE IF NOT EXISTS context69.runtime_embedding_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    provider_account_key TEXT NOT NULL REFERENCES context69.runtime_provider_accounts(account_key),
    model TEXT NOT NULL,
    dimensions BIGINT NOT NULL,
    timeout_secs BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (btrim(model) <> ''),
    CHECK (dimensions > 0),
    CHECK (timeout_secs > 0)
);

CREATE TABLE IF NOT EXISTS context69.runtime_scheduler_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    interval_secs BIGINT NOT NULL,
    run_on_start BOOLEAN NOT NULL DEFAULT TRUE,
    max_concurrency BIGINT NOT NULL,
    job_id TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (interval_secs > 0),
    CHECK (max_concurrency > 0),
    CHECK (btrim(job_id) <> '')
);

CREATE TABLE IF NOT EXISTS context69.runtime_chunking_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    max_chars BIGINT NOT NULL,
    overlap_chars BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (max_chars > 0),
    CHECK (overlap_chars >= 0),
    CHECK (overlap_chars < max_chars)
);

CREATE TABLE IF NOT EXISTS context69.runtime_file_library_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    storage_root TEXT NOT NULL,
    max_upload_size_mb BIGINT NOT NULL,
    max_upload_request_size_mb BIGINT NOT NULL,
    ingest_concurrency BIGINT NOT NULL,
    pdf_pages_per_task BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (btrim(storage_root) <> ''),
    CHECK (max_upload_size_mb > 0),
    CHECK (max_upload_request_size_mb > 0),
    CHECK (max_upload_request_size_mb >= max_upload_size_mb),
    CHECK (ingest_concurrency > 0),
    CHECK (pdf_pages_per_task > 0)
);

CREATE TABLE IF NOT EXISTS context69.runtime_source_connections (
    name TEXT PRIMARY KEY,
    database_url TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(name) <> ''),
    CHECK (btrim(database_url) <> '')
);

ALTER TABLE context69.docling_settings
    ADD COLUMN IF NOT EXISTS provider_account_key TEXT REFERENCES context69.runtime_provider_accounts(account_key);
