-- Clear the rebuild barrier of its owner, publish the new epoch and enqueue
-- its own staging snapshot for bounded cleanup.
--
-- Only the owner of the running barrier may clear it, so a superseded owner
-- writes nothing at all. The epoch moves with the clear, so a search that
-- started before the rebuild observes the change. A successful publish is the
-- only place that clears the durable derived-index invalidity: the installed
-- chunk table became the live original index in this same transaction.
--
-- The target collection is captured before the barrier drops it, and only this
-- target is enqueued. The address row is the durable progress of the existing
-- maintenance drain; no staging rows are deleted inline.
WITH target AS (
    SELECT rebuild_collection AS collection
    FROM context69.search_registry_epoch
    WHERE id = 1
),
cleared AS (
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
        rebuild_install_started = FALSE,
        rebuild_install_cursor_document_id = NULL,
        rebuild_install_cursor_chunk_index = NULL,
        rebuild_installed_chunks = 0,
        rebuild_install_total_chunks = 0,
        document_index_invalid = FALSE,
        epoch = context69.search_registry_epoch.epoch + 1,
        updated_at = now()
    WHERE id = 1
      AND rebuild_state = 'running'
      AND rebuild_owner_item = $1
      AND rebuild_owner_token = $2
    RETURNING epoch
),
enqueued AS (
    INSERT INTO context69.document_rebuild_staging_cleanup (target_collection)
    SELECT target.collection
    FROM target
    CROSS JOIN cleared
    WHERE target.collection IS NOT NULL
    ON CONFLICT (target_collection) DO NOTHING
)
SELECT epoch
FROM cleared
