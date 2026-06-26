UPDATE context69.source_configs
SET display_name = $2,
    description = $3,
    example_queries = $4,
    connection = $5,
    sync_strategy = $6,
    connector_type = $7,
    base_query = $8,
    batch_size = $9,
    updated_at = now()
WHERE source_key = $1
  AND ($10::bigint IS NULL OR project_id = $10)
