-- The producer switch of one replacement registry entry.
--
-- Only the staging -> switched edge is allowed here. The caller supplies the
-- complete old-live set as retired ids and the producer-proven original subset
-- separately. The subset predicate and the reservation bound repeat the
-- database constraint and the admission guarantee, so a raw caller cannot
-- record provenance that the stored arrays do not support or retire more ids
-- than the entry reserved. The result is a resolved switched entry with an
-- empty staged set and a reservation equal to the retired set.
UPDATE context69.search_registry_entries
SET phase = 'switched',
    retired_ids = $2::uuid[],
    retired_original_ids = $3::uuid[],
    reserved_ids = cardinality($2::uuid[])::bigint,
    staged_ids = '{}'::uuid[]
WHERE run_id = $1
  AND phase = 'staging'
  AND $3::uuid[] <@ $2::uuid[]
  AND cardinality($2::uuid[])::bigint <= reserved_ids
RETURNING run_id
