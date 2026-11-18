-- The optional, independent image embedding configuration.
--
-- There is no default external model, endpoint, credential, or collection: the
-- table starts empty, which is the disabled state. A row with `enabled = false`
-- records an explicit disable and still retains the operator's non-secret
-- configuration. The API key never lives here; the shared store owns it under
-- its own purpose. `collection_name` is a namespace/prefix for the independent
-- physical image collection: the writer appends the image shape and the
-- runtime fingerprint, so changing a model or query model never mixes vector
-- spaces and a document or code collection is never reused.
CREATE TABLE IF NOT EXISTS context69.runtime_image_embedding_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    base_url TEXT,
    collection_name TEXT,
    model TEXT,
    query_model TEXT,
    dimensions BIGINT,
    timeout_secs BIGINT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT runtime_image_embedding_settings_base_url_check
        CHECK (base_url IS NULL OR btrim(base_url) <> ''),
    CONSTRAINT runtime_image_embedding_settings_collection_name_check
        CHECK (collection_name IS NULL OR btrim(collection_name) <> ''),
    CONSTRAINT runtime_image_embedding_settings_model_check
        CHECK (model IS NULL OR btrim(model) <> ''),
    CONSTRAINT runtime_image_embedding_settings_query_model_check
        CHECK (query_model IS NULL OR btrim(query_model) <> ''),
    CONSTRAINT runtime_image_embedding_settings_dimensions_check
        CHECK (dimensions IS NULL OR dimensions > 0),
    CONSTRAINT runtime_image_embedding_settings_timeout_check
        CHECK (timeout_secs IS NULL OR timeout_secs > 0),
    CONSTRAINT runtime_image_embedding_settings_enabled_check
        CHECK (
            NOT enabled
            OR (
                base_url IS NOT NULL
                AND btrim(base_url) <> ''
                AND collection_name IS NOT NULL
                AND btrim(collection_name) <> ''
                AND model IS NOT NULL
                AND btrim(model) <> ''
                AND dimensions IS NOT NULL
                AND dimensions > 0
                AND timeout_secs IS NOT NULL
                AND timeout_secs > 0
            )
        )
);

-- Durable origin registry of written image vectors.
--
-- An origin is the exact physical point one image write produced: file id,
-- content revision, runtime fingerprint, endpoint, physical collection, and
-- the revision-aware point id. It is registered before the Qdrant write and
-- marked active only after the write acknowledges, so deletion never infers a
-- location from the current settings and an interrupted write is reconciled by
-- the durable deletion targets or the bounded writing-origin scan. Each row is
-- owned by one publish attempt id, and the point id includes that attempt, so a
-- retry or revert never reuses a point that cleanup may still delete. No
-- foreign key: the origin outlives the file document rows it describes.
CREATE TABLE IF NOT EXISTS context69.image_vector_origins (
    attempt_id UUID PRIMARY KEY,
    group_id BIGINT NOT NULL,
    file_id UUID NOT NULL,
    content_sha256 TEXT NOT NULL,
    runtime_fingerprint TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    collection_name TEXT NOT NULL,
    point_id UUID NOT NULL,
    state TEXT NOT NULL DEFAULT 'writing',
    attempt_started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT image_vector_origins_state_check
        CHECK (state IN ('writing', 'active')),
    CONSTRAINT image_vector_origins_sha_check
        CHECK (btrim(content_sha256) <> ''),
    CONSTRAINT image_vector_origins_fingerprint_check
        CHECK (btrim(runtime_fingerprint) <> ''),
    CONSTRAINT image_vector_origins_endpoint_check
        CHECK (btrim(endpoint) <> ''),
    CONSTRAINT image_vector_origins_collection_check
        CHECK (btrim(collection_name) <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_image_vector_origins_point
    ON context69.image_vector_origins (endpoint, collection_name, point_id);

CREATE INDEX IF NOT EXISTS idx_image_vector_origins_file
    ON context69.image_vector_origins (group_id, file_id);

-- Bounded reconciliation scans writing attempts that never reached their
-- acknowledgement. Active origins are never uncertain.
CREATE INDEX IF NOT EXISTS idx_image_vector_origins_writing
    ON context69.image_vector_origins (state, updated_at);

-- Durable image-vector deletion targets.
--
-- A full image file or group delete turns every recorded origin into one
-- exact target: physical endpoint, collection, and attempt-specific point id.
-- The bounded cleanup pass retries each target until the index confirms that
-- exact point is gone. A stale upsert therefore cannot survive a delete, and
-- a changed endpoint or model leaves the old origin as its own target. No
-- foreign key: the target outlives the file, document, and group rows.
CREATE TABLE IF NOT EXISTS context69.image_vector_deletions (
    id BIGSERIAL PRIMARY KEY,
    group_id BIGINT NOT NULL,
    file_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    point_id UUID NOT NULL,
    content_sha256 TEXT NOT NULL,
    runtime_fingerprint TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    collection_name TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    purged_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT image_vector_deletions_endpoint_check
        CHECK (btrim(endpoint) <> ''),
    CONSTRAINT image_vector_deletions_collection_check
        CHECK (btrim(collection_name) <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_image_vector_deletions_target
    ON context69.image_vector_deletions (endpoint, collection_name, point_id);

CREATE INDEX IF NOT EXISTS idx_image_vector_deletions_pending
    ON context69.image_vector_deletions (purged_at, updated_at);
