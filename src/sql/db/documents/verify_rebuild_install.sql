-- Fail-closed proof that the live original chunk table is exactly the staged
-- snapshot before publish.
--
-- Publish reads this once under the epoch lock: the live table must hold only
-- staged original rows, its total must equal the staged source, and every
-- staged id/document/index/text/hash must match a live row.
SELECT
    (SELECT count(*) FROM context69.document_chunks)::bigint AS "live_chunks!",
    (SELECT count(*)
     FROM context69.document_chunk_rebuild_staging
     WHERE target_collection = $1)::bigint AS "staged_chunks!",
    (SELECT count(*)
     FROM context69.document_chunk_rebuild_staging s
     LEFT JOIN context69.document_chunks c ON c.id = s.chunk_id
     WHERE s.target_collection = $1
       AND (
           c.id IS NULL
           OR c.document_id <> s.document_id
           OR c.chunk_index <> s.chunk_index
           OR c.chunk_text <> s.chunk_text
           OR c.record_hash <> s.record_hash
       ))::bigint AS "mismatch!"
