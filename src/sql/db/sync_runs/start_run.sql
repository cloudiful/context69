INSERT INTO context69.sync_runs (
    group_id,
    project_id,
    visibility,
    source_key,
    trigger_type,
    status
)
SELECT
    sc.group_id,
    sc.project_id,
    sc.visibility,
    sc.source_key,
    $2,
    'running'
FROM context69.source_configs sc
WHERE sc.source_key = $1
RETURNING id
