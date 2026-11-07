-- One stored chunk of the actual reindex fixture (issue 21 C3).
--
-- The chunk row is what the rebuild page query reads; the caller owns the
-- catalog identity so no other test row is touched.
INSERT INTO context69.document_chunks (
    id,
    document_id,
    chunk_index,
    chunk_text,
    record_hash
)
VALUES ($1, $2, $3, $4, $5)
