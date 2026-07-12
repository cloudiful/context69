INSERT INTO context69.library_file_documents (
    file_id,
    document_id,
    group_id,
    visibility,
    section_key,
    section_label,
    section_external_id,
    section_source_uri,
    section_published_at,
    section_metadata_json,
    sort_order
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
