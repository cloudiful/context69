INSERT INTO context69.groups (
    group_key,
    name,
    visibility,
    kind,
    owner_user_id,
    created_by_user_id
)
VALUES ($1, $2, 'private', 'personal', $3, $3)
RETURNING id AS "id!"
