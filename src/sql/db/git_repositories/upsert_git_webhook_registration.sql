-- Upserts the registration only when the repository is owned by $1, so one
-- group can never write a hook onto another group's repository source.
INSERT INTO context69.git_webhook_registrations (
    repository_key,
    provider_kind,
    external_hook_id,
    ownership,
    active,
    signing_secret_key
)
SELECT
    s.repository_key,
    $3,
    $4,
    $5,
    $6,
    $7
FROM context69.git_repository_sources s
WHERE s.repository_key = $1
  AND s.group_id = $2
ON CONFLICT (repository_key) DO UPDATE
SET provider_kind = EXCLUDED.provider_kind,
    external_hook_id = EXCLUDED.external_hook_id,
    ownership = EXCLUDED.ownership,
    active = EXCLUDED.active,
    signing_secret_key = EXCLUDED.signing_secret_key,
    updated_at = now()
RETURNING
    repository_key,
    provider_kind,
    external_hook_id,
    ownership,
    active,
    signing_secret_key,
    created_at,
    updated_at
