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
    NULL::smallint AS "current_role_rank?"
FROM context69.groups g
LEFT JOIN context69.groups parent ON parent.id = g.parent_group_id
WHERE g.id = $1
