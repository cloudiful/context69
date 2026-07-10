WITH default_scope AS (
    SELECT id AS group_id
    FROM context69.groups
    WHERE full_path = 'public'
)
INSERT INTO context69.source_configs (
    group_id,
    visibility,
    source_key,
    display_name,
    description,
    example_queries,
    connection,
    sync_strategy,
    connector_type,
    base_query,
    batch_size
)
SELECT
    ds.group_id,
    'public',
    $1,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    $9
FROM default_scope ds
ON CONFLICT (source_key) DO NOTHING
