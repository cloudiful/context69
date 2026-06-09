CREATE SCHEMA IF NOT EXISTS context69;

CREATE TABLE IF NOT EXISTS context69.sync_runs (
    id BIGSERIAL PRIMARY KEY,
    source_key TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    status TEXT NOT NULL,
    records_seen INTEGER NOT NULL DEFAULT 0,
    records_changed INTEGER NOT NULL DEFAULT 0,
    chunks_upserted INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sync_runs_source_started_at
    ON context69.sync_runs (source_key, started_at DESC);

CREATE TABLE IF NOT EXISTS context69.source_checkpoints (
    source_key TEXT PRIMARY KEY,
    cursor_updated_at TIMESTAMPTZ,
    cursor_external_id TEXT,
    last_success_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS context69.documents (
    id BIGSERIAL PRIMARY KEY,
    source_key TEXT NOT NULL,
    external_id TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT,
    source_uri TEXT NOT NULL,
    published_at DATE,
    updated_at_source TIMESTAMPTZ NOT NULL,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    record_hash TEXT NOT NULL,
    last_synced_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_key, external_id)
);

CREATE INDEX IF NOT EXISTS idx_documents_source_published_at
    ON context69.documents (source_key, published_at DESC);

CREATE INDEX IF NOT EXISTS idx_documents_record_hash
    ON context69.documents (record_hash);

CREATE TABLE IF NOT EXISTS context69.document_versions (
    id BIGSERIAL PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    record_hash TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT,
    body_text TEXT NOT NULL,
    source_uri TEXT NOT NULL,
    published_at DATE,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, record_hash)
);

CREATE TABLE IF NOT EXISTS context69.document_chunks (
    id UUID PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    chunk_text TEXT NOT NULL,
    record_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_document_chunks_document_id
    ON context69.document_chunks (document_id, chunk_index);

CREATE INDEX IF NOT EXISTS idx_document_chunks_record_hash
    ON context69.document_chunks (record_hash);
