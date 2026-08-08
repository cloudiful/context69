SELECT
    sc.group_id,
    g.group_key,
    g.full_path AS group_path,
    sc.visibility,
    sc.source_key,
    sc.display_name,
    sc.description,
    sc.example_queries AS "example_queries!: Json<Vec<String>>",
    sc.connection,
    sc.sync_strategy,
    sc.base_query,
    sc.batch_size,
    cp.cursor_updated_at AS last_cursor_updated_at,
    cp.cursor_external_id AS last_cursor_external_id,
    cp.last_success_at
FROM context69.source_configs sc
JOIN context69.groups g ON g.id = sc.group_id
LEFT JOIN context69.source_checkpoints cp
    ON cp.source_key = sc.source_key
WHERE $1::TEXT IS NULL
   OR sc.source_key ILIKE '%' || $1 || '%'
   OR COALESCE(sc.display_name, '') ILIKE '%' || $1 || '%'
   OR COALESCE(sc.description, '') ILIKE '%' || $1 || '%'
ORDER BY sc.source_key
LIMIT $2 OFFSET $3
