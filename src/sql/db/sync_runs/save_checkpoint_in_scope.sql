INSERT INTO context69.source_checkpoints (
    group_id,
    visibility,
    source_key,
    cursor_updated_at,
    cursor_external_id,
    last_success_at,
    updated_at
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    now(),
    now()
)
ON CONFLICT (source_key) DO UPDATE
SET group_id = EXCLUDED.group_id,
    visibility = EXCLUDED.visibility,
    cursor_updated_at = EXCLUDED.cursor_updated_at,
    cursor_external_id = EXCLUDED.cursor_external_id,
    last_success_at = now(),
    updated_at = now()
