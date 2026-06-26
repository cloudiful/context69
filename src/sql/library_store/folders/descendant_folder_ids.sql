WITH RECURSIVE descendants AS (
    SELECT id
    FROM context69.library_folders
    WHERE id = $1
    UNION ALL
    SELECT child.id
    FROM context69.library_folders child
    INNER JOIN descendants parent ON child.parent_id = parent.id
)
SELECT id AS "id!" FROM descendants
