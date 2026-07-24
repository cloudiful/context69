SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.source_configs sc
WHERE $1::TEXT IS NULL
   OR sc.source_key ILIKE '%' || $1 || '%'
   OR COALESCE(sc.display_name, '') ILIKE '%' || $1 || '%'
   OR COALESCE(sc.description, '') ILIKE '%' || $1 || '%'
