SELECT DISTINCT document_id
FROM context69.library_file_documents
WHERE file_id = ANY($1)
ORDER BY document_id
