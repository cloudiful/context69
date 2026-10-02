-- Restores one group-owned provider connection to service.
--
-- $1 group_id, $2 connection_key.
--
-- Only the lifecycle column is touched: `disabled_at` is cleared and `updated_at` is
-- stamped. The stored credential, App-private-key, and webhook-signing-secret
-- references are left exactly as they were, and this statement never reaches the
-- internal secret store and never reads a secret value, so an enable/disable cycle
-- can neither rotate nor lose a credential.
--
-- The predicate is the owning group and the key, and nothing else. It deliberately
-- does NOT require `disabled_at IS NOT NULL`: enabling an already-enabled
-- connection is the same successful match, which is what keeps a repeated enable
-- idempotent instead of a conflict. A connection of another group matches no row,
-- so the caller learns only that this group has no such key.
UPDATE context69.git_provider_connections
SET disabled_at = NULL,
    updated_at = now()
WHERE group_id = $1
  AND connection_key = $2
