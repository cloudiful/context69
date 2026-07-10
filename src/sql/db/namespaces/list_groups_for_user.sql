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
SELECT
    g.id AS "id!",
    g.parent_group_id AS "parent_group_id?",
    g.full_path AS "group_path!",
    parent.full_path AS "parent_group_path?",
    g.group_key AS "group_key!",
    g.name AS "name!",
    g.visibility AS "visibility!",
    g.kind AS "kind!",
    g.owner_user_id AS "owner_user_id?",
    g.created_at AS "created_at!",
    g.updated_at AS "updated_at!",
    gr.role_rank AS "current_role_rank?"
FROM context69.groups g
LEFT JOIN context69.groups parent ON parent.id = g.parent_group_id
LEFT JOIN group_roles gr ON gr.group_id = g.id
WHERE g.visibility = 'public' OR gr.role_rank IS NOT NULL
ORDER BY g.full_path
