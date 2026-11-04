-- Bulk-seeds unlinked domain heads for the minority-class candidate-bound
-- regression (issue 1172 P6A2).
--
-- Domain rows only: every seeded item is linked through the public bounded
-- cutover adoption API, which produces its library segment with the public
-- enqueue call. No library row is inserted or updated here.
WITH inserted AS (
    INSERT INTO context69.tasks
        (id, user_id, group_id, kind, status, group_path, total_count, queued_count, created_at)
    SELECT gen_random_uuid(), $1, NULL, $3, 'queued', 'test/admission-many',
           1, 1, now() - interval '2 days'
    FROM generate_series(1::bigint, $2)
    RETURNING id
)
INSERT INTO context69.task_items
    (id, task_id, ordinal, payload, status, created_at)
SELECT gen_random_uuid(), id, 1, '{}'::jsonb, 'queued', now() - interval '2 days'
FROM inserted
RETURNING id AS item_id, task_id
