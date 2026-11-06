-- Record that the owned rebuild target was created (issue 1212 R3).
--
-- The marker is the durable proof that the recorded target collection is the
-- one this attempt created, so a later attempt may attach it. It is written
-- immediately after a successful create and only under the exact owner: the
-- owner item, its grant token, the captured fingerprint and the recorded
-- collection are all part of the predicate, so a superseded or cancelled
-- attempt marks nothing.
--
-- `$1` is the owner item, `$2` the grant token, `$3` the captured fingerprint
-- and `$4` the recorded collection.
UPDATE context69.search_registry_epoch
SET rebuild_target_created = TRUE,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
RETURNING epoch
