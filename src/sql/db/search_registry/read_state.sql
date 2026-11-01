-- The coherent search snapshot in one statement.
--
-- The epoch, the registry entries and the deleted entity sets are read by one
-- statement, so a search attempt can never pair an epoch with a different set
-- of entries, deleted files or deleted documents.
--
-- The registry entries of a permanently tombstoned file are omitted: the file
-- entity exclusion of the same snapshot dominates every point of that file,
-- including a point a late write adds, so such an entry never contributes to
-- the visible result. The rows themselves stay in the registry as history.
SELECT
    e.epoch AS epoch,
    COALESCE(
        (
            SELECT jsonb_agg(
                jsonb_build_object(
                    'run_id', r.run_id,
                    'file_id', r.file_id,
                    'document_ids', r.document_ids,
                    'staged_ids', r.staged_ids,
                    'retired_ids', r.retired_ids,
                    'baseline_record_hash', r.baseline_record_hash,
                    'staged_record_hash', r.staged_record_hash,
                    'generation', r.generation,
                    'phase', r.phase
                ) ORDER BY r.created_at, r.run_id
            )
            FROM context69.search_registry_entries r
            WHERE NOT EXISTS (
                SELECT 1
                FROM context69.library_deleted_entities d
                WHERE d.file_id = r.file_id
            )
        ),
        '[]'::jsonb
    ) AS entries,
    COALESCE(
        (
            SELECT array_agg(d.file_id)
            FROM context69.library_deleted_entities d
            WHERE d.file_id IS NOT NULL
        ),
        ARRAY[]::uuid[]
    ) AS "deleted_file_ids!",
    COALESCE(
        (
            SELECT array_agg(d.document_id)
            FROM context69.library_deleted_entities d
            WHERE d.document_id IS NOT NULL
        ),
        ARRAY[]::bigint[]
    ) AS "deleted_document_ids!"
FROM context69.search_registry_epoch e
WHERE e.id = 1
