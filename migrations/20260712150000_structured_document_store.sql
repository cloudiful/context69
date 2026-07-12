-- Context69 structured document store.
ALTER TABLE context69.documents
    ALTER COLUMN published_at TYPE TIMESTAMPTZ
    USING CASE
        WHEN jsonb_typeof(metadata_json->'published_at') = 'string'
             AND metadata_json->>'published_at' ~ '^\d{4}-\d{2}-\d{2}T'
        THEN (metadata_json->>'published_at')::timestamptz
        ELSE published_at::timestamp AT TIME ZONE 'UTC'
    END;

ALTER TABLE context69.document_versions
    ALTER COLUMN published_at TYPE TIMESTAMPTZ
    USING CASE
        WHEN jsonb_typeof(metadata_json->'published_at') = 'string'
             AND metadata_json->>'published_at' ~ '^\d{4}-\d{2}-\d{2}T'
        THEN (metadata_json->>'published_at')::timestamptz
        ELSE published_at::timestamp AT TIME ZONE 'UTC'
    END;

CREATE TABLE context69.metadata_index_definitions (
    index_id UUID PRIMARY KEY,
    group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    field_path TEXT NOT NULL,
    data_type TEXT NOT NULL CHECK (data_type IN ('keyword', 'integer', 'float', 'boolean', 'datetime')),
    value_kind TEXT NOT NULL CHECK (value_kind IN ('scalar', 'array')),
    sortable BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL CHECK (status IN ('building', 'ready', 'failed', 'deleting')),
    processed_documents BIGINT NOT NULL DEFAULT 0,
    total_documents BIGINT NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (group_id, source_key, field_path),
    CHECK (btrim(source_key) <> ''),
    CHECK (btrim(field_path) <> ''),
    CHECK (value_kind = 'scalar' OR sortable = FALSE)
);

CREATE INDEX idx_metadata_index_definitions_pending
    ON context69.metadata_index_definitions (status, updated_at);

CREATE TABLE context69.document_metadata_values (
    index_id UUID NOT NULL REFERENCES context69.metadata_index_definitions(index_id) ON DELETE CASCADE,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL DEFAULT 0,
    keyword_value TEXT,
    integer_value BIGINT,
    float_value DOUBLE PRECISION,
    boolean_value BOOLEAN,
    datetime_value TIMESTAMPTZ,
    PRIMARY KEY (index_id, document_id, ordinal),
    CHECK (num_nonnulls(keyword_value, integer_value, float_value, boolean_value, datetime_value) = 1)
);

CREATE INDEX idx_document_metadata_keyword
    ON context69.document_metadata_values (index_id, keyword_value, document_id);
CREATE INDEX idx_document_metadata_integer
    ON context69.document_metadata_values (index_id, integer_value, document_id);
CREATE INDEX idx_document_metadata_float
    ON context69.document_metadata_values (index_id, float_value, document_id);
CREATE INDEX idx_document_metadata_boolean
    ON context69.document_metadata_values (index_id, boolean_value, document_id);
CREATE INDEX idx_document_metadata_datetime
    ON context69.document_metadata_values (index_id, datetime_value, document_id);
