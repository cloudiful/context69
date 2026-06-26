SELECT
    u.id AS "user_id!",
    u.login_name,
    u.display_name,
    gm.role
FROM context69.group_memberships gm
JOIN context69.users u ON u.id = gm.user_id
JOIN context69.groups g ON g.id = gm.group_id
WHERE g.group_key = $1
ORDER BY u.login_name
