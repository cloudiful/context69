INSERT INTO context69.tasks (
    id, user_id, group_id, kind, group_path, source_key, total_count, queued_count
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
RETURNING id
