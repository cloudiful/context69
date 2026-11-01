-- get_document_business_source.sql (issue 1138, P19)
--
-- The canonical business source of one document for a metadata publication.
-- The current version row is joined by the document's current record hash, so
-- the body used to recompute the hash is exactly the body the current hash
-- covers. A missing row yields a NULL body and the caller must fail closed
-- instead of reassembling overlapping chunks.
--
-- The document row is locked for the caller transaction: the file row is
-- already held, and this keeps the lock order file -> documents ordered by id.
SELECT
    d.title,
    d.summary,
    d.source_uri,
    d.published_at,
    d.updated_at_source,
    d.metadata_json,
    d.record_hash,
    v.body_text AS "body_text?"
FROM context69.documents d
LEFT JOIN context69.document_versions v
    ON v.document_id = d.id AND v.record_hash = d.record_hash
WHERE d.id = $1
FOR UPDATE OF d
