-- Projected registry usage after the rebuild delta entries were written.
--
-- $1 is the physical collection the rebuild is about to activate. A switched
-- replacement entry that belongs to another collection discounts exactly its
-- producer-proven original subset, because the rebuild recreates those
-- originals under its own live reservation. Translation ids, legacy rows
-- without provenance and entries that already belong to the target stay fully
-- counted. No active pointer is consulted here: the explicit target is the
-- collection that is being installed.
WITH per_entry AS (
    SELECT
        r.file_id,
        CASE
            WHEN r.phase = 'switched'
                AND cardinality(r.retired_original_ids) > 0
                AND r.collection_name IS NOT NULL
                AND r.collection_name <> $1::text
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
),
per_file AS (
    SELECT sum(reserved) AS reserved
    FROM per_entry
    WHERE file_id IS NOT NULL
    GROUP BY file_id
)
SELECT
    COALESCE((SELECT max(reserved) FROM per_file), 0)::bigint AS "max_file!",
    COALESCE((SELECT max(reserved) FROM per_entry), 0)::bigint AS "max_run!"
