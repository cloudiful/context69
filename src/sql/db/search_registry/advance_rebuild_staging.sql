-- Advance the staging cursor of one owned running rebuild.
UPDATE context69.search_registry_epoch
SET rebuild_staged_document_id = $5::bigint,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
RETURNING TRUE AS "updated!"
