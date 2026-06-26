INSERT INTO context69.document_versions (
    document_id,
    record_hash,
    title,
    summary,
    body_text,
    source_uri,
    published_at,
    metadata_json
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (document_id, record_hash) DO NOTHING
