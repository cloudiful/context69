-- Conditional lifecycle transition of one registry entry.
--
-- The `(run_id, expected phase)` predicate is the compare-and-set: an entry
-- that already moved matches no row, so no array changes and no epoch bump
-- happens. The edge guard repeats the Rust whitelist in the database, so a
-- raw caller cannot apply an illegal move either.
--
-- The three legal edges own the id arrays they leave behind:
--   staging -> switched : retired_ids becomes the caller's complete old-live
--                         set and staged_ids is cleared, because the staged
--                         ids are the live version from now on;
--   staging -> retired  : retired_ids becomes the stored staged_ids and
--                         staged_ids is cleared, so an abandoned run keeps
--                         its points excluded;
--   switched -> retired : retired_ids is retained and staged_ids is cleared.
UPDATE context69.search_registry_entries
SET phase = $3::text,
    retired_ids = CASE
        WHEN $3::text = 'switched' THEN $4::uuid[]
        WHEN $2::text = 'staging' THEN staged_ids
        ELSE retired_ids
    END,
    reserved_ids = CASE
        WHEN $3::text = 'switched' THEN cardinality($4::uuid[])::bigint
        WHEN $2::text = 'staging' THEN cardinality(staged_ids)::bigint
        ELSE cardinality(retired_ids)::bigint
    END,
    staged_ids = CASE
        WHEN $3::text IN ('switched', 'retired') THEN '{}'::uuid[]
        ELSE staged_ids
    END
WHERE run_id = $1
  AND phase = $2::text
  AND CASE
      WHEN $3::text = 'switched' THEN cardinality($4::uuid[])::bigint
      WHEN $2::text = 'staging' THEN cardinality(staged_ids)::bigint
      ELSE cardinality(retired_ids)::bigint
  END <= reserved_ids
  AND (
      ($2::text = 'staging' AND $3::text IN ('switched', 'retired'))
      OR ($2::text = 'switched' AND $3::text = 'retired')
  )
RETURNING run_id
