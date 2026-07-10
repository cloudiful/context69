WITH group_scope AS (
    SELECT id AS group_id, visibility
    FROM context69.groups
    WHERE id = $4
),
parent_scope AS (
    SELECT group_id, visibility
    FROM context69.library_folders
    WHERE id = $2 AND group_id = $4
),
resolved_scope AS (
    SELECT group_id, visibility FROM parent_scope
    UNION ALL
    SELECT group_id, visibility FROM group_scope
    LIMIT 1
)
INSERT INTO context69.library_folders (
    id,
    group_id,
    visibility,
    parent_id,
    name
)
SELECT $1, rs.group_id, rs.visibility, $2, $3
FROM resolved_scope rs
RETURNING
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = group_id) AS "group_path!",
    visibility,
    id,
    parent_id,
    name,
    created_at,
    updated_at
