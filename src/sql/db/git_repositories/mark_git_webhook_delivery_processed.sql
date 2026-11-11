-- Marks one delivery processed inside its own registration.
--
-- The same provider, hook identity, and owning source that recorded the row
-- must match, so an unprocessed delivery of another registration can never be
-- mutated through a shared provider delivery id.
--
-- $1 provider_kind, $2 external_hook_id, $3 repository_key, $4 delivery_id,
-- $5 status, $6 target_commit_sha.
UPDATE context69.git_webhook_deliveries
SET status = $5,
    target_commit_sha = COALESCE($6, target_commit_sha),
    processed_at = now()
WHERE provider_kind = $1
  AND external_hook_id = $2
  AND repository_key = $3
  AND delivery_id = $4
RETURNING delivery_id
