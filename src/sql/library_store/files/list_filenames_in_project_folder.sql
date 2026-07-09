SELECT filename
FROM context69.library_files
WHERE project_id = $1
  AND (
    ($2::uuid IS NULL AND folder_id IS NULL)
    OR folder_id = $2
  )
  AND ($3::uuid IS NULL OR id <> $3)
ORDER BY filename, id
