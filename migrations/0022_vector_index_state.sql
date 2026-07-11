CREATE TABLE IF NOT EXISTS context69.vector_index_state (
    collection_name TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    embedding_base_url TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    dimensions BIGINT NOT NULL CHECK (dimensions > 0),
    rebuilt_chunks BIGINT NOT NULL CHECK (rebuilt_chunks >= 0),
    rebuilt_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
