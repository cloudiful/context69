INSERT INTO context69.groups (
    parent_group_id,
    group_key,
    full_path,
    name,
    visibility,
    kind,
    owner_user_id,
    created_by_user_id
)
VALUES (NULL, $1, $1, $2, 'private', 'personal', $3, $3)
RETURNING id AS "id!"
