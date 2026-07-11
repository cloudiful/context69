CREATE TABLE IF NOT EXISTS context69.internal_secrets (
    key TEXT PRIMARY KEY,
    value BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
