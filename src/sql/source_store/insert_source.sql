WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
INSERT INTO context69.source_configs (
    group_id,
    project_id,
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
    ds.project_id,
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
