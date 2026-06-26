DELETE FROM context69.documents
WHERE id IN (
    SELECT DISTINCT document_id
    FROM context69.library_file_documents
    WHERE file_id = ANY($1)
)
