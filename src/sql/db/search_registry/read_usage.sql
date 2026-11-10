-- The registry admission usage of one file and the whole registry.
--
-- The entries of a permanently tombstoned file are excluded: the file entity
-- exclusion of the same snapshot dominates every point of that file, including
-- a point a late write adds, so such an entry never consumes admission budget
-- again. The predicate is the exact one of the snapshot read_state, so the
-- admission decision and the visible snapshot always agree.
--
-- A switched replacement entry keeps the ids of an old physical collection
-- excluded from search history. When the entry belongs to a different
-- collection than the one that is active now, its producer-proven original
-- subset is already recreated as live originals and is not an outstanding
-- registry reservation. Translation ids and every legacy row without proof
-- stay fully counted, and a deployment without an active pointer discounts
-- nothing.
WITH per_entry AS (
    SELECT
        r.file_id,
        r.phase,
        r.staged_ids,
        r.retired_ids,
        CASE
            WHEN r.phase = 'switched'
                AND cardinality(r.retired_original_ids) > 0
                AND r.collection_name IS NOT NULL
                AND EXISTS (
                    SELECT 1
                    FROM context69.vector_index_state v
                    WHERE v.active
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM context69.vector_index_state v
                    WHERE v.active AND v.collection_name = r.collection_name
                )
            THEN (
                SELECT count(*)::bigint
                FROM (
                    SELECT retired.id
                    FROM unnest(r.retired_ids) AS retired(id)
                    EXCEPT
                    SELECT origin.id
                    FROM unnest(r.retired_original_ids) AS origin(id)
                ) AS remaining
            )
            ELSE GREATEST(
                r.reserved_ids,
                cardinality(r.staged_ids)::bigint + cardinality(r.retired_ids)::bigint
            )
        END AS reserved
    FROM context69.search_registry_entries r
    WHERE NOT EXISTS (
        SELECT 1
        FROM context69.library_deleted_entities d
        WHERE d.file_id = r.file_id
    )
)
SELECT
    COALESCE(sum(reserved), 0)::bigint AS "total!",
    COALESCE(sum(reserved) FILTER (WHERE file_id = $1), 0)::bigint AS "file_total!",
    COALESCE(bool_or(CASE WHEN phase = 'staging' THEN staged_ids && $2::uuid[] ELSE retired_ids && $2::uuid[] END), false) AS "overlaps!"
FROM per_entry
