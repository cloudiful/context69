-- Mark staging complete and persist the exact staged total.
UPDATE context69.search_registry_epoch
SET rebuild_staging_complete = TRUE,
    rebuild_total_chunks = $5::bigint,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
  AND $5::bigint >= rebuild_processed_chunks
RETURNING TRUE AS "updated!"
