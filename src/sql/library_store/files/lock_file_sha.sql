-- Locks one library file row and returns its current content revision.
--
-- The row lock is taken under the same image group/file scopes an image
-- publish or delete uses, so a replacement decides against the revision the
-- exclusive writer protects.
--
-- $1 group_id, $2 file_id.
SELECT sha256
FROM context69.library_files
WHERE group_id = $1
  AND id = $2
FOR UPDATE
