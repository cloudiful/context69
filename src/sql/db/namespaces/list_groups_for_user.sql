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
WHERE (g.visibility = 'public' OR gr.role_rank IS NOT NULL)
  AND g.parent_group_id IS NULL
  AND (
       NULLIF(BTRIM($2::TEXT), '') IS NULL
    OR g.group_key ILIKE '%' || BTRIM($2::TEXT) || '%'
    OR g.name ILIKE '%' || BTRIM($2::TEXT) || '%'
    OR g.full_path ILIKE '%' || BTRIM($2::TEXT) || '%'
  )
  AND ($7::TEXT IS NULL OR g.visibility = $7::TEXT)
  AND ($8::TEXT IS NULL OR g.kind = $8::TEXT)
ORDER BY
    CASE WHEN $5::TEXT = 'name' AND $6::TEXT = 'asc' THEN LOWER(g.name) END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'name' AND $6::TEXT = 'desc' THEN LOWER(g.name) END DESC NULLS LAST,
    CASE WHEN $5::TEXT = 'group_key' AND $6::TEXT = 'asc' THEN g.group_key END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'group_key' AND $6::TEXT = 'desc' THEN g.group_key END DESC NULLS LAST,
    CASE WHEN $5::TEXT = 'group_path' AND $6::TEXT = 'asc' THEN g.full_path END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'group_path' AND $6::TEXT = 'desc' THEN g.full_path END DESC NULLS LAST,
    CASE WHEN $5::TEXT = 'created_at' AND $6::TEXT = 'asc' THEN g.created_at END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'created_at' AND $6::TEXT = 'desc' THEN g.created_at END DESC NULLS LAST,
    CASE WHEN $5::TEXT = 'updated_at' AND $6::TEXT = 'asc' THEN g.updated_at END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'updated_at' AND $6::TEXT = 'desc' THEN g.updated_at END DESC NULLS LAST,
    g.full_path ASC,
    g.id ASC
LIMIT $3 OFFSET $4
