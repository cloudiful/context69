-- The document-rechunk staging state of the registry epoch row.
SELECT
    rebuild_chunk_budget,
    rebuild_generation,
    rebuild_staging_complete,
    rebuild_staged_document_id,
    rebuild_translation_cursor_document_id,
    rebuild_translation_cursor_translation_id,
    rebuild_translation_cursor_chunk_index,
    rebuild_translation_processed_chunks
FROM context69.search_registry_epoch
WHERE id = 1
