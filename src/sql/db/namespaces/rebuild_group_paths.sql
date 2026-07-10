WITH RECURSIVE group_paths AS (
    SELECT
        g.id,
        g.parent_group_id,
        g.group_key,
        g.group_key::text AS full_path
    FROM context69.groups g
    WHERE g.parent_group_id IS NULL

    UNION ALL

    SELECT
        child.id,
        child.parent_group_id,
        child.group_key,
        group_paths.full_path || '/' || child.group_key
    FROM context69.groups child
    JOIN group_paths ON child.parent_group_id = group_paths.id
)
UPDATE context69.groups g
SET full_path = group_paths.full_path
FROM group_paths
WHERE g.id = group_paths.id
