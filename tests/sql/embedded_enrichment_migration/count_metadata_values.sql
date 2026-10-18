SELECT count(*) AS "count!"
FROM context69.document_metadata_values
WHERE index_id = ANY($1)
