-- Post-conflict classification for the legacy-path migration: distinguishes a
-- row another worker already linked (storage_object_id set) from a row that
-- was concurrently replaced under the same id.
SELECT storage_object_id,
       storage_rel_path
FROM context69.library_files
WHERE id = $1
