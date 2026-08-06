SELECT
    d.id,
    g.group_key,
    g.full_path AS group_path,
    g.visibility,
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
WHERE d.id = ANY($1)
  AND (g.visibility = 'public' OR d.group_id = ANY($2))
  AND ($3::text IS NULL OR g.full_path = $3)
