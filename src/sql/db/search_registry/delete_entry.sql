-- Conditional removal of one registry entry.
--
-- Only an empty staging row may be removed. A staging row that still holds
-- ids belongs to a run that has not resolved, and a switched or retired row
-- is the tombstone of an already resolved run; both keep their ids excluded
-- from the search paths, so removing them would leak a hidden point back into
-- the search results.
DELETE FROM context69.search_registry_entries
WHERE run_id = $1
  AND phase = 'staging'
  AND cardinality(staged_ids) = 0
  AND cardinality(retired_ids) = 0
RETURNING run_id
