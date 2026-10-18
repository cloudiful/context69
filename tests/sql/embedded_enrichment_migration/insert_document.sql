INSERT INTO context69.documents
    (group_id, source_key, external_id, title, summary, source_uri,
     updated_at_source, record_hash, metadata_json, visibility)
VALUES ($1, 'retire-migration-src', $2, 'Business Doc', NULL,
        'https://example.test/retire-migration', now(), $3, $4, 'public')
RETURNING id
