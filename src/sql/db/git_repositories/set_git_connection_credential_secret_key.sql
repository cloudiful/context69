-- Narrows the read-credential reference of one group-owned connection.
--
-- The broad connection upsert is deliberately not reused for this: its conflict
-- clause clears `disabled_at`, so sealing or rotating a credential through it
-- would silently re-enable a connection an operator had disabled. Sealing a
-- secret is not a lifecycle change, so this statement touches the reference and
-- `updated_at` only.
--
-- The write is conditional on the owning group, so a connection of another
-- group matches no row and the caller learns the update did not land instead of
-- having repointed someone else's credential reference.
UPDATE context69.git_provider_connections
SET credential_secret_key = $3,
    updated_at = now()
WHERE group_id = $1
  AND connection_key = $2
