SELECT file_id, document_id, group_id, visibility, section_key, section_label,
       section_external_id, section_source_uri, section_published_at, section_metadata_json,
       sort_order
FROM context69.library_file_documents
WHERE file_id = $1
ORDER BY sort_order ASC, section_key ASC
