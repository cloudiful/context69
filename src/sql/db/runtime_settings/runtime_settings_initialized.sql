SELECT EXISTS(
    SELECT 1
    FROM context69.runtime_qdrant_settings
    WHERE singleton = TRUE
) AS "initialized!"
