SELECT url, collection_name, recreate_on_dimension_mismatch
FROM context69.runtime_qdrant_settings
WHERE singleton = TRUE
