-- One queued staging target to drain, under the epoch lock.
--
-- The recorded target of the running or failed barrier is never drained: its
-- staged rows are the recoverable progress of an attempt that may resume.
SELECT cleanup.target_collection
FROM context69.document_rebuild_staging_cleanup cleanup
WHERE cleanup.target_collection IS DISTINCT FROM (
    SELECT CASE
        WHEN epoch.rebuild_state IN ('running', 'failed') THEN epoch.rebuild_collection
        ELSE NULL
    END
    FROM context69.search_registry_epoch epoch
    WHERE epoch.id = 1
)
ORDER BY cleanup.target_collection
LIMIT 1
