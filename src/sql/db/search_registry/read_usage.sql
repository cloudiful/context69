-- The registry admission usage of one file and the whole registry.
--
-- The entries of a permanently tombstoned file are excluded: the file entity
-- exclusion of the same snapshot dominates every point of that file, including
-- a point a late write adds, so such an entry never consumes admission budget
-- again. The predicate is the exact one of the snapshot read_state, so the
-- admission decision and the visible snapshot always agree.
SELECT
    COALESCE(sum(GREATEST(reserved_ids, cardinality(staged_ids)::bigint + cardinality(retired_ids)::bigint)), 0)::bigint AS "total!",
    COALESCE(sum(GREATEST(reserved_ids, cardinality(staged_ids)::bigint + cardinality(retired_ids)::bigint)) FILTER (WHERE file_id = $1), 0)::bigint AS "file_total!",
    COALESCE(bool_or(CASE WHEN phase = 'staging' THEN staged_ids && $2::uuid[] ELSE retired_ids && $2::uuid[] END), false) AS "overlaps!"
FROM context69.search_registry_entries r
WHERE NOT EXISTS (
    SELECT 1
    FROM context69.library_deleted_entities d
    WHERE d.file_id = r.file_id
)
