-- Desired runtime vector identity of a test deployment (issue 1212 P4a).
--
-- The qdrant settings and the embedding settings are one coherent desired
-- configuration, so a fixture writes both in one statement. Existing values
-- are replaced: a case owns the singletons it seeds.
WITH qdrant AS (
    INSERT INTO context69.runtime_qdrant_settings (singleton, url, collection_name)
    VALUES (TRUE, $1, $2)
    ON CONFLICT (singleton) DO UPDATE
    SET url = EXCLUDED.url,
        collection_name = EXCLUDED.collection_name
    RETURNING singleton
)
INSERT INTO context69.runtime_embedding_settings
    (singleton, base_url, model, dimensions, timeout_secs)
VALUES (TRUE, $3, $4, $5, 30)
ON CONFLICT (singleton) DO UPDATE
SET base_url = EXCLUDED.base_url,
    model = EXCLUDED.model,
    dimensions = EXCLUDED.dimensions
