-- Record a failed rebuild attempt.
--
-- The owner stays recorded with its identity, so a retry of the same identity
-- may take the barrier over again while a different identity is refused.
-- Only the owner of the running barrier may fail it.
UPDATE context69.search_registry_epoch
SET rebuild_state = 'failed',
    rebuild_error = $3,
    -- A failed rebuild keeps its committed cursor and counters: the same
    -- identity may resume after the last written page. The pause flag cannot
    -- outlive the running state.
    rebuild_pause_requested = FALSE,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
RETURNING epoch
