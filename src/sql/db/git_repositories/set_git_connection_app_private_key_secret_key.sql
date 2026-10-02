-- Narrows the GitHub App private-key reference of one group-owned connection.
--
-- The App key is its own secret with its own purpose and lifecycle, so it moves
-- on its own reference column and is never carried in a broad connection
-- update. `disabled_at` is left alone for the same reason the credential
-- reference is: sealing a key is not a lifecycle change, and this statement
-- must not re-enable a disabled connection.
--
-- The write is conditional on the owning group, so a connection of another
-- group matches no row and the caller learns the update did not land.
UPDATE context69.git_provider_connections
SET app_private_key_secret_key = $3,
    updated_at = now()
WHERE group_id = $1
  AND connection_key = $2
