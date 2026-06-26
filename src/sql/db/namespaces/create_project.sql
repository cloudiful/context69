INSERT INTO context69.projects (
    group_id,
    project_key,
    name,
    visibility,
    created_by_user_id
)
VALUES ($1, $2, $3, $4, $5)
RETURNING id AS "id!"
