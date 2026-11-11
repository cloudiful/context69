-- Shared ancestor-key lock for one source registration (issue #28 phase P13).
--
-- A source insert into a group that a concurrent delete would cascade must not
-- slip past that delete's ownership check. This statement takes a shared
-- advisory lock on the target group and every ancestor, in the same transaction
-- that inserts the source. A group delete takes the exclusive lock on its own
-- key first, so the two orders cannot deadlock: either the registration commits
-- before the delete counts, or it waits and then its insert fails because the
-- cascaded group is gone.
--
-- $1 group_id.
WITH RECURSIVE ancestors AS (
    SELECT g.id, g.parent_group_id
    FROM context69.groups g
    WHERE g.id = $1::bigint

    UNION ALL

    SELECT parent.id, parent.parent_group_id
    FROM context69.groups parent
    JOIN ancestors child ON child.parent_group_id = parent.id
)
SELECT pg_advisory_xact_lock_shared(
    hashtextextended('context69.git_group_ownership:' || ancestors.id::text, 0)
)
FROM ancestors
