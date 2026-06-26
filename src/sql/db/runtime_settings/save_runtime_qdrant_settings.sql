INSERT INTO context69.runtime_qdrant_settings (
    singleton,
    url,
    collection_name,
    recreate_on_dimension_mismatch,
    updated_at
)
VALUES (TRUE, $1, $2, $3, now())
ON CONFLICT (singleton) DO UPDATE
SET url = EXCLUDED.url,
    collection_name = EXCLUDED.collection_name,
    recreate_on_dimension_mismatch = EXCLUDED.recreate_on_dimension_mismatch,
    updated_at = now()
