SELECT
    c.id AS chunk_id,
    d.id AS document_id,
    d.group_id,
    g.group_key,
    g.full_path AS group_path,
    d.visibility,
    d.source_key,
    d.external_id,
    d.title,
    d.summary,
    d.source_uri,
    d.published_at,
    d.updated_at_source,
    d.record_hash,
    c.chunk_index,
    c.chunk_text,
    d.metadata_json
FROM context69.document_chunks c
INNER JOIN context69.documents d ON d.id = c.document_id
INNER JOIN context69.groups g ON g.id = d.group_id
INNER JOIN context69.library_file_documents lfd ON lfd.document_id = d.id
WHERE lfd.file_id = ANY($1)
ORDER BY d.id, c.chunk_index
