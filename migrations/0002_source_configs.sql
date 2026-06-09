CREATE TABLE IF NOT EXISTS context69.source_configs (
    source_key TEXT PRIMARY KEY,
    connection TEXT NOT NULL,
    sync_strategy TEXT NOT NULL,
    connector_type TEXT NOT NULL,
    base_query TEXT NOT NULL,
    batch_size BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (batch_size > 0)
);

CREATE INDEX IF NOT EXISTS idx_source_configs_connection
    ON context69.source_configs (connection);
