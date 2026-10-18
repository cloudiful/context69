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
VALUES ($1, 'public', $2, $2, NULL, '[]'::jsonb, $3, $4, 'postgres_sql', $5, $6)
