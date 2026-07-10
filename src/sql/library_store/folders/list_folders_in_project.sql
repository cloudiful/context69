SELECT
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = group_id) AS "group_path!",
    visibility,
    id,
    parent_id,
    name,
    created_at,
    updated_at
FROM context69.library_folders
WHERE group_id = $1
ORDER BY name, id
