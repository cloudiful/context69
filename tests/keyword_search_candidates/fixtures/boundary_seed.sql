WITH inserted_documents AS (
    INSERT INTO context69.documents (
        group_id, source_key, external_id, title, summary, source_uri,
        published_at, updated_at_source, metadata_json, record_hash, visibility
    )
    SELECT $1,
           'fixture-source',
           'boundary-' || sequence,
           'Boundary ' || sequence,
           NULL,
           'https://example.test/issue62/boundary/' || sequence,
           now() - (90 - sequence) * interval '1 second',
           now(),
           '{}'::jsonb,
           $2::text || ':' || sequence,
           'public'
    FROM generate_series(1, 90) AS boundary(sequence)
    RETURNING id, external_id
), inserted_chunks AS (
    INSERT INTO context69.document_chunks (id, document_id, chunk_index, chunk_text, record_hash)
    SELECT md5(external_id)::uuid,
           id,
           0,
           $2::text || ' boundaryneedle ' || external_id,
           $2::text || ':' || external_id
    FROM inserted_documents
    RETURNING id
)
SELECT count(*)::bigint FROM inserted_chunks
