DO $$
BEGIN
    CREATE EXTENSION IF NOT EXISTS pg_trgm;
EXCEPTION
    WHEN insufficient_privilege THEN
        RAISE NOTICE 'pg_trgm extension not created; keyword search will use sequential ILIKE fallback';
END
$$;

CREATE TABLE IF NOT EXISTS context69.search_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    mode TEXT NOT NULL DEFAULT 'hybrid',
    rerank_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    rerank_base_url TEXT NOT NULL DEFAULT 'https://openrouter.ai/api/v1',
    rerank_model TEXT NOT NULL DEFAULT 'cohere/rerank-4-fast',
    candidate_limit BIGINT NOT NULL DEFAULT 40,
    timeout_secs BIGINT NOT NULL DEFAULT 10,
    api_key TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (mode IN ('vector', 'hybrid')),
    CHECK (btrim(rerank_base_url) <> ''),
    CHECK (btrim(rerank_model) <> ''),
    CHECK (candidate_limit > 0),
    CHECK (timeout_secs > 0)
);

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_trgm') THEN
        EXECUTE 'CREATE INDEX IF NOT EXISTS documents_title_trgm_idx ON context69.documents USING gin (lower(title) gin_trgm_ops)';
        EXECUTE 'CREATE INDEX IF NOT EXISTS document_chunks_text_trgm_idx ON context69.document_chunks USING gin (lower(chunk_text) gin_trgm_ops)';
    END IF;
END
$$;
