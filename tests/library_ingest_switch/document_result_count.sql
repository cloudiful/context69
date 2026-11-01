-- The number of appended results of one document.
SELECT COUNT(*) AS "count!"
FROM context69.document_results
WHERE document_id = $1
