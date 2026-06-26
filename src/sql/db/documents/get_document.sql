SELECT
    d.id,
    g.group_key,
    p.project_key,
    d.visibility,
    d.source_key,
    d.external_id,
    d.title,
    d.summary,
    d.source_uri,
    d.published_at,
    d.updated_at_source,
    d.record_hash,
    d.metadata_json
FROM context69.documents d
INNER JOIN context69.groups g ON g.id = d.group_id
INNER JOIN context69.projects p ON p.id = d.project_id
WHERE d.id = $1
  AND (d.visibility = 'public' OR d.project_id = ANY($2))
  AND ($3::text IS NULL OR g.group_key = $3)
  AND ($4::text IS NULL OR p.project_key = $4)
