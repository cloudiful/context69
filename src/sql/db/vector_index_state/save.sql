INSERT INTO context69.vector_index_state (
    collection_name,
    fingerprint,
    embedding_base_url,
    embedding_model,
    dimensions,
    rebuilt_chunks
)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (collection_name) DO UPDATE
SET fingerprint = EXCLUDED.fingerprint,
    embedding_base_url = EXCLUDED.embedding_base_url,
    embedding_model = EXCLUDED.embedding_model,
    dimensions = EXCLUDED.dimensions,
    rebuilt_chunks = EXCLUDED.rebuilt_chunks,
    rebuilt_at = NOW()
