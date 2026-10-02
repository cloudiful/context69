SELECT
    s.group_id,
    r.repository_key,
    r.provider_kind,
    r.external_hook_id,
    r.ownership,
    r.active,
    r.signing_secret_key,
    r.created_at,
    r.updated_at
FROM context69.git_webhook_registrations r
JOIN context69.git_repository_sources s ON s.repository_key = r.repository_key
WHERE r.provider_kind = $1
  AND r.external_hook_id = $2
