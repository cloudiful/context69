-- The persistent rebuild barrier of the registry epoch row.
--
-- The barrier is read under the epoch row lock, so a caller that locked the
-- epoch before it reads the barrier sees the state of the same instant as
-- every other writer.
SELECT
    rebuild_state,
    rebuild_owner_item,
    rebuild_owner_token,
    rebuild_fingerprint,
    rebuild_collection,
    rebuild_started_at,
    rebuild_error,
    rebuild_pause_requested,
    rebuild_cursor_document_id,
    rebuild_cursor_chunk_index,
    rebuild_processed_chunks,
    rebuild_total_chunks,
    rebuild_target_created
FROM context69.search_registry_epoch
WHERE id = 1
