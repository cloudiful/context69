INSERT INTO context69.documents (
    source_key,
    group_id,
    visibility,
    external_id,
    title,
    source_uri,
    updated_at_source,
    record_hash
)
VALUES ($1, $2, 'public', $3, $3, $4, now(), $5)
RETURNING id
