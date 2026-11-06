-- Clears the pause request of one owned running rebuild (issue 1212 P3).
--
-- The caller has already requeued the exact parked item in the same
-- transaction, so the barrier and the item agree when this commits. The barrier
-- keeps its owner, its identity and its committed cursor: a resume continues
-- after the last written page, never from the beginning. The epoch does not move
-- because the registry's visible entry set does not change.
UPDATE context69.search_registry_epoch
SET rebuild_pause_requested = FALSE,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_pause_requested
RETURNING rebuild_pause_requested AS "pause_requested!"
