SELECT file_id, document_id, group_id, project_id, visibility, section_key, section_label, sort_order
FROM context69.library_file_documents
WHERE file_id = $1
ORDER BY sort_order ASC, section_key ASC
