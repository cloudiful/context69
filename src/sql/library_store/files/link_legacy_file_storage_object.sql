-- Conditional reference update for the legacy-path migration. The update only
-- lands while the row still points at the exact old direct path and has no
-- storage object, so a concurrent replacement or a parallel migration worker
-- can never be overwritten.
UPDATE context69.library_files
SET storage_object_id = $2,
    storage_rel_path = $3,
    updated_at = now()
WHERE id = $1
  AND storage_rel_path = $4
  AND storage_object_id IS NULL
RETURNING id
