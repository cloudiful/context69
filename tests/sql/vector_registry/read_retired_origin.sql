-- The producer-proven retired origin of one replacement registry entry.
--
-- Shared by the real switch and rebuild quota fixtures so both observe the
-- stored provenance and reservation of the same transition instead of a
-- fabricated entry shape.
SELECT retired_original_ids, reserved_ids
FROM context69.search_registry_entries
WHERE run_id = $1
