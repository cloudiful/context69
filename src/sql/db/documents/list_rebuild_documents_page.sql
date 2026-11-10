-- One page of current documents with their canonical version body.
--
-- Paged by document id, never by chunk rows: a document whose body produces
-- zero chunks must still appear so its progress row advances.
SELECT
    d.id AS document_id,
    d.group_id,
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
    d.metadata_json,
    v.body_text
FROM context69.documents d
INNER JOIN context69.groups g ON g.id = d.group_id
INNER JOIN context69.document_versions v
    ON v.document_id = d.id
   AND v.record_hash = d.record_hash
WHERE $1::BIGINT IS NULL OR d.id > $1
ORDER BY d.id
LIMIT $2
