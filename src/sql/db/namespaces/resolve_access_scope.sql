WITH RECURSIVE inherited_groups AS (
    SELECT
        g.id,
        CASE gm.role
            WHEN 'owner' THEN 3
            WHEN 'maintainer' THEN 2
            ELSE 1
        END::smallint AS role_rank
    FROM context69.group_memberships gm
    JOIN context69.groups g ON g.id = gm.group_id
    WHERE gm.user_id = $1

    UNION ALL

    SELECT
        child.id,
        inherited_groups.role_rank
    FROM context69.groups child
    JOIN inherited_groups ON child.parent_group_id = inherited_groups.id
),
group_roles AS (
    SELECT id AS group_id, MAX(role_rank)::smallint AS role_rank
    FROM inherited_groups
    GROUP BY id
)
SELECT g.id AS "group_id!"
FROM context69.groups g
LEFT JOIN group_roles gr ON gr.group_id = g.id
WHERE g.visibility = 'private'
  AND gr.role_rank IS NOT NULL
  AND ($2::text IS NULL OR g.full_path = $2)
ORDER BY g.id
