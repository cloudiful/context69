-- Remove exactly the registry entries one fixture created.
--
-- The caller supplies the run ids it owns, so teardown never deletes a row
-- another fixture owns in the reused issue test database.
DELETE FROM context69.search_registry_entries
WHERE run_id = ANY($1::uuid[])
