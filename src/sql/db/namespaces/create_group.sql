WITH parent_scope AS (
    SELECT id, full_path
    FROM context69.groups
    WHERE id = $1
),
resolved_scope AS (
    SELECT
        $1::bigint AS parent_group_id,
        CASE
            WHEN $1::bigint IS NULL THEN $2::text
            ELSE (SELECT full_path || '/' || $2 FROM parent_scope)
        END AS full_path
)
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
SELECT parent_group_id, $2, full_path, $3, $4, $5, $6, $6
FROM resolved_scope
RETURNING id AS "id!"
