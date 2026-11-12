-- Release the barrier of a rebuild that never installed anything.
--
-- Unlike a successful publish, this path must not touch the durable
-- derived-index invalidity: a resumed attempt that fails before its target is
-- prepared may already describe a partially installed index, and clearing the
-- flag here would make that partial table searchable. The committed staging and
-- install checkpoints are kept for the retry; only the ownership and the
-- recorded target are released, and the released target is enqueued for the
-- bounded staging drain.
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
        rebuild_target_created = FALSE,
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
