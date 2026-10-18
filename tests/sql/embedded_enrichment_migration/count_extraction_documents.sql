SELECT count(*) AS "count!"
FROM context69.documents
WHERE metadata_json ? 'extraction'
