-- The actor a background refresh is attributed to.
--
-- A manager whose registration created the source is preferred, then the
-- group owner, then the first current member. No row means the group has no
-- user that can own a task, so the scheduler reports the source as blocked
-- instead of inventing an actor.
--
-- $1 group_id, $2 preferred_user_id.
SELECT
    COALESCE(
        $2,
        g.owner_user_id,
        (
            SELECT gm.user_id
            FROM context69.group_memberships gm
            WHERE gm.group_id = g.id
            ORDER BY
                CASE gm.role
                    WHEN 'owner' THEN 0
                    WHEN 'maintainer' THEN 1
                    ELSE 2
                END,
                gm.user_id
            LIMIT 1
        )
    ) AS user_id
FROM context69.groups g
WHERE g.id = $1
