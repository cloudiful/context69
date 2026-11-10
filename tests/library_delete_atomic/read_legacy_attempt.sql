-- The owned tombstone's purge state for the legacy-origin case.
--
-- An unknown-origin row must stay pending and must never be claimed: the claim
-- stamps last_purge_attempt_at before any network call, so a NULL attempt is
-- direct proof that no remote delete was attempted for this row.
SELECT
    purged_at IS NULL AS "unpurged!",
    last_purge_attempt_at IS NULL AS "unattempted!"
FROM context69.library_deleted_entities
WHERE id = $1
