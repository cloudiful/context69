SELECT d.id AS document_id, d.group_id, g.group_key, g.full_path AS group_path,
       d.visibility, d.source_key, d.external_id, d.source_uri, d.published_at,
       d.updated_at_source, d.metadata_json, d.record_hash, d.title, d.summary, v.body_text
FROM context69.documents d
JOIN context69.groups g ON g.id = d.group_id
JOIN context69.document_versions v
  ON v.document_id = d.id AND v.record_hash = d.record_hash
WHERE d.id = $1

