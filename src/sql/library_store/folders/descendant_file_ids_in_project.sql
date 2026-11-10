-- Every file under one project-scoped folder subtree, on the caller's
-- transaction.
WITH RECURSIVE descendants AS (
    SELECT id
    FROM context69.library_folders
    WHERE group_id = $1
      AND id = $2
    UNION
    SELECT child.id
    FROM context69.library_folders child
    INNER JOIN descendants parent ON child.parent_id = parent.id
    WHERE child.group_id = $1
)
SELECT file.id AS "id!"
FROM context69.library_files file
INNER JOIN descendants ON descendants.id = file.folder_id
WHERE file.group_id = $1
ORDER BY file.id
