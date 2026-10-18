SELECT count(*) AS "count!"
FROM context69.metadata_index_definitions
WHERE index_id = ANY($1)
