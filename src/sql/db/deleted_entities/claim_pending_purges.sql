-- Claim a bounded set of actionable tombstones for one purge pass.
--
-- Only known-origin, unpurged rows enter the attempt queue. The row lock plus
-- SKIP LOCKED lets another worker claim a different set without waiting, and
-- the attempt stamp commits with the claim, before any network call. An origin
-- that fails or is unreachable therefore moves to the back of the ordering
-- instead of blocking later known rows. Unknown origins are excluded and stay
-- in the raw pending/reporting list.
WITH claimed AS (
    SELECT id
    FROM context69.library_deleted_entities
    WHERE purged_at IS NULL
      AND target_endpoint IS NOT NULL
      AND target_collection IS NOT NULL
    ORDER BY COALESCE(last_purge_attempt_at, deleted_at), id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
UPDATE context69.library_deleted_entities entity
SET last_purge_attempt_at = now()
FROM claimed
WHERE entity.id = claimed.id
RETURNING
    entity.id,
    entity.file_id,
    entity.document_id,
    entity.group_id,
    entity.target_endpoint,
    entity.target_collection,
    entity.deleted_at,
    entity.purged_at
