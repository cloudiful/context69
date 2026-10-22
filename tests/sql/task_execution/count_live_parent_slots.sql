SELECT count(*) AS "count!" FROM context69.tasks WHERE lease_token IS NOT NULL AND lease_until > now() AND status IN ('queued', 'running', 'waiting') AND deleted_at IS NULL
