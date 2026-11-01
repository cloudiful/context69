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
    rebuild_started_at = now(),
    rebuild_error = NULL,
    updated_at = now()
WHERE id = 1
RETURNING epoch
