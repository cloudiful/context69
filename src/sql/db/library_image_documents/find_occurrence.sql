-- The image of one occurrence slot, read inside the locked link transaction.
--
-- $1 group_id, $2 document_id, $3 role, $4 ordinal.
SELECT file_id, document_id, group_id, role, ordinal, caption, context_text,
       source_uri, created_at, updated_at
FROM context69.library_image_documents
WHERE group_id = $1
  AND document_id = $2
  AND role = $3
  AND ordinal = $4
