-- Claim the rebuild barrier for one fenced owner.
--
-- The epoch row is locked by the caller before this statement runs, so the
-- compare-and-set never races another writer: the row it updates is the row
-- the caller already holds.
UPDATE context69.search_registry_epoch
SET rebuild_state = 'running',
    rebuild_owner_item = $1,
    rebuild_owner_token = $2,
    rebuild_fingerprint = $3,
    rebuild_collection = $4,
    rebuild_started_at = COALESCE(rebuild_started_at, now()),
    rebuild_error = NULL,
    -- A fresh idle admission starts from the beginning. A same-identity
    -- takeover of a running or failed barrier keeps the committed cursor,
    -- so a resumed rebuild continues after its last written page.
    rebuild_pause_requested = CASE
        WHEN rebuild_state = 'idle' THEN FALSE
        ELSE rebuild_pause_requested
    END,
    rebuild_cursor_document_id = CASE
        WHEN rebuild_state = 'idle' THEN NULL
        ELSE rebuild_cursor_document_id
    END,
    rebuild_cursor_chunk_index = CASE
        WHEN rebuild_state = 'idle' THEN NULL
        ELSE rebuild_cursor_chunk_index
    END,
    rebuild_processed_chunks = CASE
        WHEN rebuild_state = 'idle' THEN 0
        ELSE rebuild_processed_chunks
    END,
    rebuild_total_chunks = CASE
        WHEN rebuild_state = 'idle' THEN 0
        ELSE rebuild_total_chunks
    END,
    -- The ownership marker of the recorded target. A fresh idle admission
    -- names a target that no attempt has created yet, so the marker starts
    -- false. A same-identity takeover keeps the marker of the recorded
    -- target: it still describes the very collection that attempt created.
    rebuild_target_created = CASE
        WHEN rebuild_state = 'idle' THEN FALSE
        ELSE rebuild_target_created
    END,
    updated_at = now()
WHERE id = 1
RETURNING epoch
