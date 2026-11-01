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
    epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
RETURNING epoch
