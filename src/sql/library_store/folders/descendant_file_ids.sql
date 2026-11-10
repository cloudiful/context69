-- Every file under one folder subtree, on the caller's transaction.
WITH RECURSIVE descendants AS (
    SELECT id
    FROM context69.library_folders
    WHERE id = $1
    UNION
    SELECT child.id
    FROM context69.library_folders child
    INNER JOIN descendants parent ON child.parent_id = parent.id
)
SELECT file.id AS "id!"
FROM context69.library_files file
INNER JOIN descendants ON descendants.id = file.folder_id
ORDER BY file.id
