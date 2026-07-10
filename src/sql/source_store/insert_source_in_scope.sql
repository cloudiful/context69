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
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
