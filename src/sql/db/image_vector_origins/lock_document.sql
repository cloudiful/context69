-- One document scope of the shared image-vector mutation lock.
--
-- Callers take the group scope first, then every file scope in sorted order,
-- then every document scope in sorted order.
--
-- $1 document_id.
SELECT pg_advisory_xact_lock(
    hashtextextended('context69.image_vector_document:' || $1::bigint::text, 0)
)
