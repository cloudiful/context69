-- Exclusive ownership coordination for one group move (issue #28 phase P13).
--
-- Re-parenting a subtree changes which delete would cascade it. The move takes
-- the exclusive ownership lock on the moved group and every ancestor of both
-- its old and new locations, so it serializes with:
--   * source registration, which holds shared locks on the same keys;
--   * group deletion, which holds the exclusive lock on the deleting group.
--
-- The keys are deduplicated and ordered by id so concurrent moves that overlap
-- request the shared keys in the same order. No global tree or hierarchy lock
-- is taken.
--
-- $1 moved_group_id, $2 target_parent_group_id (nullable for a root move).
WITH RECURSIVE old_chain AS (
    SELECT g.id, g.parent_group_id
    FROM context69.groups g
    WHERE g.id = $1::bigint

    UNION ALL

    SELECT parent.id, parent.parent_group_id
    FROM context69.groups parent
    JOIN old_chain child ON child.parent_group_id = parent.id
),
new_chain AS (
    SELECT g.id, g.parent_group_id
    FROM context69.groups g
    WHERE g.id = $2::bigint

    UNION ALL

    SELECT parent.id, parent.parent_group_id
    FROM context69.groups parent
    JOIN new_chain child ON child.parent_group_id = parent.id
),
keys AS (
    SELECT id FROM old_chain
    UNION
    SELECT id FROM new_chain
)
SELECT pg_advisory_xact_lock(
    hashtextextended('context69.git_group_ownership:' || ordered.id::text, 0)
)
FROM (SELECT id FROM keys ORDER BY id) ordered
