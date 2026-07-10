UPDATE context69.documents
SET title = $3,
    summary = $4,
    source_uri = $5,
    published_at = $6,
    updated_at_source = COALESCE($7, updated_at_source),
    metadata_json = $8,
    record_hash = $9,
    last_synced_at = now(),
    updated_at = now()
WHERE group_id = $1 AND source_key = $2 AND external_id = $10
