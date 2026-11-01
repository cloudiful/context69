-- The documents of every file in a folder subtree.
--
-- The folder delete removes the files of the subtree, so their documents are
-- removed in the same statement: a document of a file that joined the subtree
-- after the enumeration is removed too, and no document of a deleted file
-- survives as an orphan.
WITH RECURSIVE subtree AS (
    SELECT id
    FROM context69.library_folders
    WHERE id = $1
      AND group_id = $2
    UNION ALL
    SELECT child.id
    FROM context69.library_folders child
    JOIN subtree parent ON child.parent_id = parent.id
    WHERE child.group_id = $2
)
DELETE FROM context69.documents
WHERE id IN (
    SELECT DISTINCT link.document_id
    FROM context69.library_file_documents link
    JOIN context69.library_files file ON file.id = link.file_id
    WHERE file.folder_id IN (SELECT id FROM subtree)
)
