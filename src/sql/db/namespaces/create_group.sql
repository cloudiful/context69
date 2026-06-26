INSERT INTO context69.groups (
    parent_group_id,
    group_key,
    name,
    visibility,
    kind,
    owner_user_id,
    created_by_user_id
)
VALUES ($1, $2, $3, $4, $5, $6, $6)
RETURNING id AS "id!"
