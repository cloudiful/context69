-- Mark the derived original index invalid before its first destructive reset.
--
-- The epoch bump commits with the flag, so every search attempt that snapshotted
-- the previous epoch retries and then observes the barrier or the invalidity.
-- The predicate is the exact owner fence: a superseded or cancelled attempt
-- marks nothing. An already invalid index returns no row, which the caller
-- treats as "still invalid".
UPDATE context69.search_registry_epoch
SET document_index_invalid = TRUE,
    epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
  AND NOT document_index_invalid
RETURNING epoch
