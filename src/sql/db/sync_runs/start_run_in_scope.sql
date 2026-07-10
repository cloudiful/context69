INSERT INTO context69.sync_runs (
    group_id,
    visibility,
    source_key,
    trigger_type,
    status
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    'running'
)
RETURNING id
