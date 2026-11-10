-- Records a provider delivery under its receiving registration.
--
-- The provider delivery id is unique only inside one registration, identified
-- by provider, provider-issued hook id, and owning repository source, so a
-- delivery from another registration with the same id inserts its own row. A
-- redelivery of this registration's id inserts nothing and returns the original
-- row.
--
-- $1 delivery_id, $2 external_hook_id, $3 provider_kind, $4 repository_key,
-- $5 status, $6 target_commit_sha.
INSERT INTO context69.git_webhook_deliveries (
    delivery_id,
    external_hook_id,
    provider_kind,
    repository_key,
    status,
    target_commit_sha
)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (provider_kind, external_hook_id, repository_key, delivery_id)
DO UPDATE
SET delivery_id = context69.git_webhook_deliveries.delivery_id
RETURNING
    delivery_id,
    external_hook_id,
    provider_kind,
    repository_key,
    status,
    target_commit_sha,
    received_at,
    processed_at
