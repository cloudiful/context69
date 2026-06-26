SELECT
    c.id AS chunk_id,
    d.id AS document_id,
    g.group_key,
    p.project_key,
    d.visibility,
    d.source_key,
    d.external_id,
    d.title,
    d.summary,
    d.source_uri,
    d.published_at,
    c.chunk_index,
    c.chunk_text,
    d.metadata_json
FROM context69.document_chunks c
INNER JOIN context69.documents d ON d.id = c.document_id
INNER JOIN context69.groups g ON g.id = d.group_id
INNER JOIN context69.projects p ON p.id = d.project_id
WHERE c.id = ANY($1)
  AND (d.visibility = 'public' OR d.project_id = ANY($2))
  AND ($3::text IS NULL OR g.group_key = $3)
  AND ($4::text IS NULL OR p.project_key = $4)
