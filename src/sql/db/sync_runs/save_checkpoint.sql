INSERT INTO context69.source_checkpoints (
    group_id,
    project_id,
    visibility,
    source_key,
    cursor_updated_at,
    cursor_external_id,
    last_success_at,
    updated_at
)
SELECT
    sc.group_id,
    sc.project_id,
    sc.visibility,
    sc.source_key,
    $2,
    $3,
    now(),
    now()
FROM context69.source_configs sc
WHERE sc.source_key = $1
ON CONFLICT (source_key) DO UPDATE
SET cursor_updated_at = EXCLUDED.cursor_updated_at,
    cursor_external_id = EXCLUDED.cursor_external_id,
    last_success_at = now(),
    updated_at = now()
