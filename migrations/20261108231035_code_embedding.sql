-- The optional, independent Git code embedding configuration.
--
-- There is no default external model, endpoint, or credential: the table starts
-- empty, which is the disabled state. A row with `enabled = false` records an
-- explicit disable and still retains the operator's non-secret configuration.
-- The API key never lives here; the shared store owns it under its own purpose.
CREATE TABLE IF NOT EXISTS context69.runtime_code_embedding_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    base_url TEXT,
    model TEXT,
    query_model TEXT,
    dimensions BIGINT,
    timeout_secs BIGINT,
    max_items_per_request BIGINT,
    max_item_chars BIGINT,
    max_request_chars BIGINT,
    max_request_bytes BIGINT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT runtime_code_embedding_settings_base_url_check
        CHECK (base_url IS NULL OR btrim(base_url) <> ''),
    CONSTRAINT runtime_code_embedding_settings_model_check
        CHECK (model IS NULL OR btrim(model) <> ''),
    CONSTRAINT runtime_code_embedding_settings_query_model_check
        CHECK (query_model IS NULL OR btrim(query_model) <> ''),
    CONSTRAINT runtime_code_embedding_settings_dimensions_check
        CHECK (dimensions IS NULL OR dimensions > 0),
    CONSTRAINT runtime_code_embedding_settings_timeout_check
        CHECK (timeout_secs IS NULL OR timeout_secs > 0),
    CONSTRAINT runtime_code_embedding_settings_batching_check
        CHECK (
            (max_items_per_request IS NULL OR max_items_per_request > 0)
            AND (max_item_chars IS NULL OR max_item_chars > 0)
            AND (max_request_chars IS NULL OR max_request_chars > 0)
            AND (max_request_bytes IS NULL OR max_request_bytes > 0)
        ),
    CONSTRAINT runtime_code_embedding_settings_enabled_check
        CHECK (
            NOT enabled
            OR (
                base_url IS NOT NULL
                AND btrim(base_url) <> ''
                AND model IS NOT NULL
                AND btrim(model) <> ''
                AND dimensions IS NOT NULL
                AND dimensions > 0
                AND timeout_secs IS NOT NULL
                AND timeout_secs > 0
            )
        )
);

-- The per-generation code vector metadata that P2 writes after all points are
-- upserted and before the generation is activated. P1 creates the table only:
-- no code path reads or writes it in this phase.
CREATE TABLE IF NOT EXISTS context69.git_generation_code_vectors (
    generation_key UUID PRIMARY KEY
        REFERENCES context69.git_repository_generations(generation_key) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL CHECK (btrim(fingerprint) <> ''),
    dimensions BIGINT NOT NULL CHECK (dimensions > 0),
    collection_name TEXT NOT NULL CHECK (btrim(collection_name) <> ''),
    point_count BIGINT NOT NULL DEFAULT 0 CHECK (point_count >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
