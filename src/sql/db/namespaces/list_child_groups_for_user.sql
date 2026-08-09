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
JOIN context69.groups parent ON parent.id = g.parent_group_id
LEFT JOIN group_roles gr ON gr.group_id = g.id
WHERE parent.full_path = $2
  AND (g.visibility = 'public' OR gr.role_rank IS NOT NULL)
  AND (
       NULLIF(BTRIM($3::TEXT), '') IS NULL
    OR g.group_key ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR g.name ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR g.full_path ILIKE '%' || BTRIM($3::TEXT) || '%'
  )
ORDER BY
    CASE WHEN $6::TEXT = 'name' AND $7::TEXT = 'asc' THEN LOWER(g.name) END ASC NULLS LAST,
    CASE WHEN $6::TEXT = 'name' AND $7::TEXT = 'desc' THEN LOWER(g.name) END DESC NULLS LAST,
    CASE WHEN $6::TEXT = 'group_key' AND $7::TEXT = 'asc' THEN g.group_key END ASC NULLS LAST,
    CASE WHEN $6::TEXT = 'group_key' AND $7::TEXT = 'desc' THEN g.group_key END DESC NULLS LAST,
    CASE WHEN $6::TEXT = 'group_path' AND $7::TEXT = 'asc' THEN g.full_path END ASC NULLS LAST,
    CASE WHEN $6::TEXT = 'group_path' AND $7::TEXT = 'desc' THEN g.full_path END DESC NULLS LAST,
    CASE WHEN $6::TEXT = 'created_at' AND $7::TEXT = 'asc' THEN g.created_at END ASC NULLS LAST,
    CASE WHEN $6::TEXT = 'created_at' AND $7::TEXT = 'desc' THEN g.created_at END DESC NULLS LAST,
    CASE WHEN $6::TEXT = 'updated_at' AND $7::TEXT = 'asc' THEN g.updated_at END ASC NULLS LAST,
    CASE WHEN $6::TEXT = 'updated_at' AND $7::TEXT = 'desc' THEN g.updated_at END DESC NULLS LAST,
    g.group_key ASC,
    g.id ASC
LIMIT $4 OFFSET $5
