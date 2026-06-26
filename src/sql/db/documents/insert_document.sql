INSERT INTO context69.documents (
    group_id,
    project_id,
    visibility,
    source_key,
    external_id,
    title,
    summary,
    source_uri,
    published_at,
    updated_at_source,
    metadata_json,
    record_hash
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
RETURNING id AS "id!"
