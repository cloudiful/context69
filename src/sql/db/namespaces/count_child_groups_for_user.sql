WITH RECURSIVE inherited_groups AS (
    SELECT g.id, CASE gm.role WHEN 'owner' THEN 3 WHEN 'maintainer' THEN 2 ELSE 1 END::smallint AS role_rank
    FROM context69.group_memberships gm
    JOIN context69.groups g ON g.id = gm.group_id
    WHERE gm.user_id = $1
    UNION ALL
    SELECT child.id, inherited_groups.role_rank
    FROM context69.groups child
    JOIN inherited_groups ON child.parent_group_id = inherited_groups.id
), group_roles AS (
    SELECT id AS group_id, MAX(role_rank)::smallint AS role_rank
    FROM inherited_groups GROUP BY id
)
SELECT COUNT(*)::BIGINT AS "count!"
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
  AND ($4::TEXT IS NULL OR g.visibility = $4::TEXT)
  AND ($5::TEXT IS NULL OR g.kind = $5::TEXT)
