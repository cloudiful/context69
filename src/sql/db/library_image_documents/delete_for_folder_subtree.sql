-- The image/document links of every document in a folder subtree.
--
-- The folder delete removes the documents of the subtree; the links to those
-- documents are removed first so no occurrence points at a missing parent.
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
DELETE FROM context69.library_image_documents link
USING context69.library_file_documents file_link,
      context69.library_files file
WHERE file_link.document_id = link.document_id
  AND file.id = file_link.file_id
  AND file.folder_id IN (SELECT id FROM subtree)
