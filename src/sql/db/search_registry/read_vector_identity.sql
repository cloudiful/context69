-- The persisted vector identity of the deployment.
--
-- The configured identity is the authority of the vector space: the switch
-- compares the identity an entry was admitted under with this one, so a node
-- whose own snapshot predates a settings change on another node cannot make
-- its points visible.
SELECT
    q.url AS "qdrant_url!",
    q.collection_name AS "collection_name!",
    e.base_url AS "embedding_base_url!",
    e.model AS "embedding_model!",
    e.dimensions AS "dimensions!"
FROM context69.runtime_qdrant_settings q
JOIN context69.runtime_embedding_settings e ON e.singleton = TRUE
WHERE q.singleton = TRUE
