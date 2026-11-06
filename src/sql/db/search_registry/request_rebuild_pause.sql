-- Requests a pause of one owned rebuild at its next batch boundary
-- (issue 1212 P3).
--
-- A pause never interrupts the current page: the flag is durable, and the
-- worker observes it after the page it already committed. Only the owner of the
-- running barrier may set it, so a pause can never target a superseded attempt
-- or a newer unseen task.
UPDATE context69.search_registry_epoch
SET rebuild_pause_requested = TRUE,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
RETURNING rebuild_pause_requested AS "pause_requested!"
