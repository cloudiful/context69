-- update_library_business_fields.sql (issue 50, phase 2)
--
-- Keep the parent document rewrite unconditional: extracted metadata publishing
-- may legitimately change external_id/source_uri/published_at/updated_at_source
-- /metadata_json even when the record_hash already matches. The chunk
-- record_hash is rewritten only when it actually changed, so a same-hash
-- publish becomes a true no-op for document_chunks instead of an unconditional
-- per-chunk rewrite that hits every row of a multi-hundred-chunk document.
-- record_hash is NOT NULL on document_chunks, so the null-safe
-- `IS DISTINCT FROM` still treats NULLs as equal (the existing rows hold a
-- hash from a previous successful publish) and only re-writes rows whose hash
-- diverges.
WITH updated_document AS (
    UPDATE context69.documents
    SET external_id = $2,
        source_uri = $3,
        published_at = $4,
        updated_at_source = $5,
        metadata_json = $6,
        record_hash = $7,
        updated_at = now()
    WHERE id = $1
    RETURNING id
)
UPDATE context69.document_chunks
SET record_hash = $7
WHERE document_id IN (SELECT id FROM updated_document)
  AND record_hash IS DISTINCT FROM $7
