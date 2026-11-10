-- Permanent tombstones for the still-existing documents among the given ids.
--
-- The group is read from the document row so the caller never supplies an
-- unverified group. A repeated id, or an id whose document no longer exists,
-- inserts nothing; the caller learns from the returned rows whether a new
-- exclusion (and one generation bump) is owed. Every row of one cleanup shares
-- the same captured physical target, which stays immutable on conflict.
INSERT INTO context69.library_deleted_entities (
    document_id, group_id, target_endpoint, target_collection
)
SELECT document.id, document.group_id, $2, $3
FROM context69.documents AS document
WHERE document.id = ANY($1::bigint[])
ON CONFLICT (document_id) WHERE document_id IS NOT NULL DO NOTHING
RETURNING id
