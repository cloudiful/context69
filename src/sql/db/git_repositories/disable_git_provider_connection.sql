UPDATE context69.git_provider_connections
SET disabled_at = COALESCE(disabled_at, now()),
    updated_at = now()
WHERE group_id = $1
  AND connection_key = $2
