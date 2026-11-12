-- Begin the one-time bounded installation of a verified staged snapshot.
--
-- Runs after the old original chunks were cleaned and truncated, under the
-- exact owner fence. The total is pinned from the staged original source, so a
-- later publish compares against the same number the installation commits.
-- The reset is only admitted once: a resumed attempt keeps its committed
-- install checkpoints.
UPDATE context69.search_registry_epoch
SET rebuild_install_started = TRUE,
    rebuild_install_cursor_document_id = NULL,
    rebuild_install_cursor_chunk_index = NULL,
    rebuild_installed_chunks = 0,
    rebuild_install_total_chunks = (
        SELECT count(*)::bigint
        FROM context69.document_chunk_rebuild_staging staging
        WHERE staging.target_collection = $4
    ),
    epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
  AND rebuild_staging_complete
  AND NOT rebuild_install_started
RETURNING rebuild_install_total_chunks
