-- Record and activate the physical collection of a test vector index (issue
-- 1212 P4a).
--
-- The row is inactive first and becomes the one active pointer. The caller
-- clears any previous active row in a separate statement, so the partial unique
-- index never observes two active rows.
INSERT INTO context69.vector_index_state (
    collection_name,
    fingerprint,
    embedding_base_url,
    embedding_model,
    dimensions,
    rebuilt_chunks,
    active
)
VALUES ($1, $2, $3, $4, $5, 0, TRUE)
ON CONFLICT (collection_name) DO UPDATE
SET fingerprint = EXCLUDED.fingerprint,
    embedding_base_url = EXCLUDED.embedding_base_url,
    embedding_model = EXCLUDED.embedding_model,
    dimensions = EXCLUDED.dimensions,
    rebuilt_chunks = EXCLUDED.rebuilt_chunks,
    active = TRUE,
    rebuilt_at = NOW()
