SELECT cursor_updated_at, cursor_external_id
FROM context69.source_checkpoints
WHERE source_key = $1
