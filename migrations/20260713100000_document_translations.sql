CREATE TABLE context69.translation_provider_settings (
    provider_key TEXT PRIMARY KEY,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    priority INTEGER NOT NULL,
    endpoint TEXT,
    api_key TEXT,
    model TEXT,
    llm_api_kind TEXT,
    deepl_plan TEXT,
    monthly_character_limit BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_translation_provider_key CHECK (
        provider_key IN ('deepl', 'llm', 'libretranslate')
    ),
    CONSTRAINT chk_translation_llm_api_kind CHECK (
        llm_api_kind IS NULL OR llm_api_kind IN (
            'openai_responses', 'openai_chat_completions', 'anthropic_messages'
        )
    ),
    CONSTRAINT chk_translation_deepl_plan CHECK (
        deepl_plan IS NULL OR deepl_plan IN ('free', 'pro')
    ),
    CONSTRAINT chk_translation_monthly_limit CHECK (
        monthly_character_limit IS NULL OR monthly_character_limit > 0
    )
);

INSERT INTO context69.translation_provider_settings (
    provider_key, enabled, priority, deepl_plan, monthly_character_limit
) VALUES
    ('deepl', FALSE, 10, 'free', 1000000),
    ('llm', FALSE, 20, NULL, NULL),
    ('libretranslate', FALSE, 30, NULL, NULL)
ON CONFLICT (provider_key) DO NOTHING;

CREATE TABLE context69.translation_provider_usage (
    provider_key TEXT NOT NULL REFERENCES context69.translation_provider_settings(provider_key),
    usage_month DATE NOT NULL,
    character_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider_key, usage_month),
    CONSTRAINT chk_translation_usage_nonnegative CHECK (character_count >= 0)
);

CREATE TABLE context69.group_translation_settings (
    group_id BIGINT PRIMARY KEY REFERENCES context69.groups(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    default_target_locales TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    source_locale TEXT,
    glossary JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_group_translation_glossary_array CHECK (jsonb_typeof(glossary) = 'array')
);

ALTER TABLE context69.library_files
    ADD COLUMN translation_override BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN translation_source_locale TEXT,
    ADD COLUMN translation_target_locales TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];

CREATE TABLE context69.document_translation_jobs (
    id UUID PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    target_locale TEXT NOT NULL,
    requested_source_locale TEXT,
    detected_source_locale TEXT,
    source_record_hash TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    provider_key TEXT,
    provider_config_hash TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    source_character_count BIGINT NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_document_translation_job_status CHECK (
        status IN ('queued', 'running', 'succeeded', 'failed', 'skipped', 'quota_exceeded')
    )
);

CREATE UNIQUE INDEX uq_document_translation_jobs_active
    ON context69.document_translation_jobs (document_id, target_locale, source_record_hash)
    WHERE status IN ('queued', 'running');

CREATE INDEX idx_document_translation_jobs_pending
    ON context69.document_translation_jobs (status, created_at, id)
    WHERE status IN ('queued', 'running');

CREATE TABLE context69.document_translation_attempts (
    id BIGSERIAL PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES context69.document_translation_jobs(id) ON DELETE CASCADE,
    provider_key TEXT NOT NULL,
    provider_config_hash TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    status TEXT NOT NULL,
    character_count BIGINT NOT NULL,
    latency_ms BIGINT NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_translation_attempt_status CHECK (
        status IN ('succeeded', 'failed', 'quota_exceeded')
    )
);

CREATE TABLE context69.document_translation_versions (
    id UUID PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    target_locale TEXT NOT NULL,
    source_locale TEXT,
    source_record_hash TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    provider_config_hash TEXT NOT NULL,
    model_name TEXT,
    translated_title TEXT NOT NULL,
    translated_summary TEXT,
    translated_body_text TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, target_locale, source_record_hash)
);

CREATE TABLE context69.document_translation_chunks (
    id UUID PRIMARY KEY,
    translation_id UUID NOT NULL REFERENCES context69.document_translation_versions(id) ON DELETE CASCADE,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    target_locale TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    chunk_text TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (translation_id, chunk_index)
);

CREATE INDEX idx_document_translation_chunks_document_locale
    ON context69.document_translation_chunks (document_id, target_locale, chunk_index);

CREATE INDEX document_translation_chunks_text_trgm_idx
    ON context69.document_translation_chunks USING gin (lower(chunk_text) gin_trgm_ops);
