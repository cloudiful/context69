INSERT INTO context69.tasks (
    id, user_id, group_id, kind, group_path, source_key, total_count, queued_count, stage
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $7,
    CASE $4
        WHEN 'url_batch' THEN 'download'
        WHEN 'file_batch' THEN 'storage'
        WHEN 'text_batch' THEN 'storage'
        WHEN 'source_sync' THEN 'sync'
        WHEN 'delete_batch' THEN 'delete'
        WHEN 'translation' THEN 'translation'
        WHEN 'vector_rebuild' THEN 'indexing'
        ELSE 'finalize'
    END
)
RETURNING id
