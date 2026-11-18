DELETE FROM context69.library_image_documents l
USING context69.library_file_documents d
WHERE l.group_id = $2
  AND l.document_id = d.document_id
  AND d.file_id = $1
