-- Fail-closed verification of the staged snapshot before activation.
SELECT
    (SELECT count(*)
     FROM context69.document_rebuild_staging_documents s
     LEFT JOIN context69.documents d ON d.id = s.document_id
     WHERE s.target_collection = $1
       AND (d.id IS NULL OR d.record_hash <> s.record_hash))::bigint
        AS "document_hash_mismatch!",
    (SELECT count(*)
     FROM context69.document_rebuild_staging_documents s
     WHERE s.target_collection = $1
       AND s.chunk_count <> (
           SELECT count(*)
           FROM context69.document_chunk_rebuild_staging c
           WHERE c.target_collection = s.target_collection
             AND c.document_id = s.document_id
       ))::bigint AS "chunk_count_mismatch!",
    (SELECT count(*)
     FROM context69.document_chunk_rebuild_staging c
     INNER JOIN context69.document_rebuild_staging_documents s
        ON s.target_collection = c.target_collection
       AND s.document_id = c.document_id
     WHERE c.target_collection = $1
       AND c.record_hash <> s.record_hash)
    + (SELECT count(*)
       FROM context69.document_translation_rebuild_staging t
       INNER JOIN context69.documents d ON d.id = t.document_id
       WHERE t.target_collection = $1
         AND t.record_hash <> d.record_hash)::bigint AS "chunk_hash_mismatch!",
    (SELECT count(*)
     FROM context69.document_rebuild_staging_documents
     WHERE target_collection = $1)::bigint AS "staged_documents!",
    (SELECT count(*)
     FROM context69.documents d
     WHERE NOT EXISTS (
         SELECT 1
         FROM context69.document_rebuild_staging_documents s
         WHERE s.target_collection = $1
           AND s.document_id = d.id))::bigint AS "document_membership_mismatch!"
