WITH RECURSIVE descendants AS (
    SELECT id
    FROM context69.library_folders
    WHERE project_id = $1
      AND id = $2
    UNION ALL
    SELECT child.id
    FROM context69.library_folders child
    INNER JOIN descendants parent ON child.parent_id = parent.id
    WHERE child.project_id = $1
)
SELECT id AS "id!" FROM descendants
