-- The persistent rebuild barrier and install state of the registry epoch row.
--
-- The barrier is read under the epoch row lock, so a caller that locked the
-- epoch before it reads the barrier sees the state of the same instant as
-- every other writer. `document_index_invalid` is the durable derived-index
-- validity and the install columns are the committed bounded-install position.
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
    rebuild_target_created,
    document_index_invalid,
    rebuild_install_started,
    rebuild_install_cursor_document_id,
    rebuild_install_cursor_chunk_index,
    rebuild_installed_chunks,
    rebuild_install_total_chunks
FROM context69.search_registry_epoch
WHERE id = 1
