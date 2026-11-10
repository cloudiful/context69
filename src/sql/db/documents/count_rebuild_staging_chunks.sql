-- Exact staged totals: fresh originals and preserved translations.
SELECT
    (SELECT count(*)
     FROM context69.document_chunk_rebuild_staging
     WHERE target_collection = $1)::bigint AS "original!",
    (SELECT count(*)
     FROM context69.document_translation_rebuild_staging
     WHERE target_collection = $1)::bigint AS "translated!"
