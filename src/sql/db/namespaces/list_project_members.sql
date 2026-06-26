SELECT
    u.id AS "user_id!",
    u.login_name,
    u.display_name,
    pm.role
FROM context69.project_memberships pm
JOIN context69.projects p ON p.id = pm.project_id
JOIN context69.groups g ON g.id = p.group_id
JOIN context69.users u ON u.id = pm.user_id
WHERE g.group_key = $1
  AND p.project_key = $2
ORDER BY u.login_name
