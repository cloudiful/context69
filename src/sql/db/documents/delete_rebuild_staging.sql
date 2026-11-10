-- Delete every staged row of one completed or discarded target.
WITH deleted_original_chunks AS (
    DELETE FROM context69.document_chunk_rebuild_staging
    WHERE target_collection = $1
),
deleted_translations AS (
    DELETE FROM context69.document_translation_rebuild_staging
    WHERE target_collection = $1
)
DELETE FROM context69.document_rebuild_staging_documents
WHERE target_collection = $1
