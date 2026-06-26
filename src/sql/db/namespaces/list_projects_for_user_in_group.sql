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
),
project_roles AS (
    SELECT
        project_id,
        MAX(
            CASE role
                WHEN 'owner' THEN 3
                WHEN 'maintainer' THEN 2
                ELSE 1
            END
        )::smallint AS role_rank
    FROM context69.project_memberships
    WHERE user_id = $1
    GROUP BY project_id
)
SELECT
    p.id AS "id!",
    p.group_id AS "group_id!",
    g.group_key AS "group_key!",
    p.project_key AS "project_key!",
    p.name AS "name!",
    p.visibility AS "visibility!",
    p.created_at AS "created_at!",
    p.updated_at AS "updated_at!",
    GREATEST(COALESCE(gr.role_rank, 0), COALESCE(pr.role_rank, 0))::smallint
        AS "current_role_rank?"
FROM context69.projects p
JOIN context69.groups g ON g.id = p.group_id
LEFT JOIN group_roles gr ON gr.group_id = p.group_id
LEFT JOIN project_roles pr ON pr.project_id = p.id
WHERE (
    p.visibility = 'public'
    OR gr.role_rank IS NOT NULL
    OR pr.role_rank IS NOT NULL
)
  AND g.group_key = $2
ORDER BY g.group_key, p.project_key
