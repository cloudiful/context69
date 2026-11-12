-- Commit one installed page: cursor, installed count and epoch move together.
--
-- The expected installed count is part of the predicate, so a replayed or
-- duplicated batch that did not observe the committed cursor increments
-- nothing. The upper bound of the pinned total is enforced both by the
-- predicate and by the epoch row constraint.
UPDATE context69.search_registry_epoch
SET rebuild_install_cursor_document_id = $5::bigint,
    rebuild_install_cursor_chunk_index = $6::integer,
    rebuild_installed_chunks = rebuild_installed_chunks + $7::bigint,
    epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
  AND rebuild_install_started
  AND $7::bigint > 0
  AND rebuild_installed_chunks = $8::bigint
  AND rebuild_installed_chunks + $7::bigint <= rebuild_install_total_chunks
RETURNING rebuild_installed_chunks
