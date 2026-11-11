-- The previous active identity, read in the activation transaction before the
-- replacement is published.
--
-- The embedding fields and the optional pinned endpoint are needed to prove
-- which endpoint owns the physical collection that is about to be retired. The
-- physical name is never used to reconstruct a legacy fingerprint.
SELECT
    collection_name,
    fingerprint,
    embedding_base_url,
    embedding_model,
    dimensions,
    qdrant_url
FROM context69.vector_index_state
WHERE active
