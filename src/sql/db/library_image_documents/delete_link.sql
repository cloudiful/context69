-- Delete one scoped occurrence link inside the locked transaction.
--
-- $1 group_id, $2 file_id, $3 document_id, $4 role, $5 ordinal.
DELETE FROM context69.library_image_documents
WHERE group_id = $1
  AND file_id = $2
  AND document_id = $3
  AND role = $4
  AND ordinal = $5
