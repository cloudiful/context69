INSERT INTO context69.documents
    (group_id, source_key, external_id, title, summary, source_uri,
     updated_at_source, record_hash, metadata_json, visibility)
VALUES ($1, 'payload-sync-src', $2, 'Payload Doc', NULL,
        'https://example.test/payload-sync', now(), $3, '{}'::jsonb, 'public')
RETURNING id
