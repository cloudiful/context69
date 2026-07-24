SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.metadata_index_definitions
WHERE group_id = $1 AND source_key = $2
