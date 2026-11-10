-- Verify one checkpointed chunk batch after its idempotent insert.
--
-- Every expected id must exist for this document with the exact index, text
-- and record hash of the batch. A primary-key conflict alone is not proof that
-- the batch is complete or that it belongs to the same segmentation, so a
-- mismatch refuses the batch before its checkpoint can advance.
WITH expected AS (
    SELECT *
    FROM unnest($2::uuid[], $3::integer[], $4::text[], $5::text[])
        AS input(chunk_id, chunk_index, chunk_text, record_hash)
)
SELECT COUNT(*)::bigint AS "count!"
FROM expected
LEFT JOIN context69.document_chunks chunk ON chunk.id = expected.chunk_id
WHERE chunk.id IS NULL
   OR chunk.document_id <> $1
   OR chunk.chunk_index <> expected.chunk_index
   OR chunk.chunk_text <> expected.chunk_text
   OR chunk.record_hash <> expected.record_hash
