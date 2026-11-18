-- The shared image-vector mutation lock of one group.
--
-- Image link writes, image vector writes, and every file, document, folder,
-- and group delete that can remove an image take this advisory lock in their
-- transaction. The lock is acquired before the file/document advisory scopes
-- and before the affected row locks, so the same deterministic order is used
-- everywhere and a link or point write can never commit into a deletion.
--
-- $1 group_id.
SELECT pg_advisory_xact_lock(
    hashtextextended('context69.image_vector_group:' || $1::bigint::text, 0)
)
