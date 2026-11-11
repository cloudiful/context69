-- The document progress of one running rebuild target.
--
-- Staged counts the documents already cut and staged into the target. Total
-- counts the canonical documents the rebuild selects, so a document without a
-- canonical body keeps the progress short of completion instead of reporting a
-- false 100%.
SELECT
    (SELECT count(*)
     FROM context69.document_rebuild_staging_documents
     WHERE target_collection = $1)::bigint AS "staged!",
    (SELECT count(*)
     FROM context69.documents)::bigint AS "total!"
