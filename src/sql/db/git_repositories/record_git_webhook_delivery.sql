INSERT INTO context69.git_webhook_deliveries (
    delivery_id,
    provider_kind,
    repository_key,
    status,
    target_commit_sha
)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (delivery_id) DO UPDATE
SET delivery_id = context69.git_webhook_deliveries.delivery_id
RETURNING
    delivery_id,
    provider_kind,
    repository_key,
    status,
    target_commit_sha,
    received_at,
    processed_at
