SELECT field_path AS "field_path!"
FROM context69.metadata_index_definitions
WHERE group_id = $1
ORDER BY field_path
