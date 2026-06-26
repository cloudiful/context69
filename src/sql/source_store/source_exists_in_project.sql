SELECT EXISTS(
    SELECT 1
    FROM context69.source_configs
    WHERE source_key = $1
      AND ($2::bigint IS NULL OR project_id = $2)
) AS "exists!"
