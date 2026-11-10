-- Clear the rebuild barrier of its owner and publish the new epoch.
--
-- Only the owner of the running barrier may clear it, so a superseded owner
-- writes nothing at all. The epoch moves with the clear, so a search that
-- started before the rebuild observes the change.
UPDATE context69.search_registry_epoch
SET rebuild_state = 'idle',
    rebuild_owner_item = NULL,
    rebuild_owner_token = NULL,
    rebuild_fingerprint = NULL,
    rebuild_collection = NULL,
    rebuild_started_at = NULL,
    rebuild_error = NULL,
    rebuild_pause_requested = FALSE,
    rebuild_cursor_document_id = NULL,
    rebuild_cursor_chunk_index = NULL,
    rebuild_processed_chunks = 0,
    rebuild_total_chunks = 0,
    rebuild_target_created = FALSE,
    rebuild_chunk_budget = NULL,
    rebuild_generation = NULL,
    rebuild_staging_complete = FALSE,
    rebuild_staged_document_id = NULL,
    rebuild_translation_cursor_document_id = NULL,
    rebuild_translation_cursor_translation_id = NULL,
    rebuild_translation_cursor_chunk_index = NULL,
    rebuild_translation_processed_chunks = 0,
    epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
RETURNING epoch
