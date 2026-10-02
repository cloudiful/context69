-- Inserts the registration only when the repository is owned by $2, so one
-- group can never write a hook onto another group's repository source.
--
-- Create-only: unlike the broad upsert this statement has no conflict clause, so
-- a repository that already has a registration and a hook identity another
-- repository already claims both raise their unique violation, which the caller
-- maps to the one bounded conflict. A create therefore can never overwrite or
-- repoint a registration another request owns.
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
RETURNING
    repository_key,
    provider_kind,
    external_hook_id,
    ownership,
    active,
    signing_secret_key,
    created_at,
    updated_at
