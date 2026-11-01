-- The number of unresolved staging entries of the registry.
--
-- A staging entry is an admitted run that never resolved, so its points are
-- not the live version yet and a rebuild would race it. The predicate is the
-- exact one of the snapshot read_state, so the rebuild and the visible
-- snapshot always agree about which entries are still unresolved. The entries
-- of a permanently tombstoned file are excluded, because their file entity
-- exclusion already dominates every point of that file.
SELECT count(*)::bigint AS "staging_count!"
FROM context69.search_registry_entries r
WHERE r.phase = 'staging'
  AND NOT EXISTS (
      SELECT 1
      FROM context69.library_deleted_entities d
      WHERE d.file_id = r.file_id
  )
