SELECT
    g.id AS "group_id!",
    g.full_path AS "group_path!",
    gm.role
FROM context69.groups g
JOIN context69.group_memberships gm
    ON gm.group_id = g.id AND gm.user_id = $1
WHERE g.kind = 'personal'
  AND g.owner_user_id = $1
ORDER BY g.id
LIMIT 1
