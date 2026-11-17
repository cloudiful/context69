-- One file scope of the shared image-vector mutation lock.
--
-- Callers take the group scope first, then every file scope in sorted order,
-- then every document scope in sorted order.
--
-- $1 file_id.
SELECT pg_advisory_xact_lock(
    hashtextextended('context69.image_vector_file:' || $1::uuid::text, 0)
)
