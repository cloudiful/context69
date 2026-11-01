-- The identity locks of one library file creation (issue 1138 P22).
--
-- Two writers that would create the same library identity must not both
-- create a row. The identity of a file in a group is its external id and its
-- content digest, so both keys are locked. The keys are taken in one
-- canonical (ascending) order, so two concurrent writers serialize on the
-- same lock instead of deadlocking on each other, and the second one
-- re-probes the identity after the first one committed.
--
-- `$1` is the group plus external id key, `$2` the group plus digest key.
WITH ordered AS MATERIALIZED (
    SELECT key
    FROM (VALUES (hashtextextended($1::text, 0)), (hashtextextended($2::text, 0)))
        AS candidates(key)
    ORDER BY key
)
SELECT count(*)::bigint AS "locked!"
FROM (SELECT pg_advisory_xact_lock(key) FROM ordered) AS taken;
