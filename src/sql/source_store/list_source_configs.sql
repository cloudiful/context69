SELECT
    sc.source_key,
    display_name,
    description,
    example_queries AS "example_queries!: Json<Vec<String>>",
    connection,
    sync_strategy,
    connector_type,
    base_query,
    batch_size
FROM context69.source_configs sc
ORDER BY sc.source_key
