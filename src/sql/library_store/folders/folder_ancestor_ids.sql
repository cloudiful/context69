-- Every ancestor of one folder, including the folder itself.
--
-- The move guard locks the source and the candidate parent first, then asks
-- whether the parent is already an ancestor of the source. A cycle would make
-- that condition true, and the distinct-set recursion also terminates on a
-- pre-existing malformed tree instead of walking forever.
WITH RECURSIVE ancestors AS (
    SELECT id, parent_id
    FROM context69.library_folders
    WHERE id = $1
    UNION
    SELECT parent.id, parent.parent_id
    FROM context69.library_folders parent
    INNER JOIN ancestors child ON child.parent_id = parent.id
)
SELECT id AS "id!" FROM ancestors
