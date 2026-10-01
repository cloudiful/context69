UPDATE context69.git_webhook_deliveries
SET status = $2,
    target_commit_sha = COALESCE($3, target_commit_sha),
    processed_at = now()
WHERE delivery_id = $1
RETURNING delivery_id
