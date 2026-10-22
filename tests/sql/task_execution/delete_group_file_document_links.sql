DELETE FROM context69.library_file_documents
WHERE file_id IN (SELECT id FROM context69.library_files WHERE group_id = $1)
