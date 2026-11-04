-- Seed one task with the seven historical-row shapes the failure-budget
-- migration must classify. Every row is created inside the caller's rollback
-- transaction, so nothing survives the test.
--
-- `$1` is the fixture task and parent id. The seeded `execution_failure_count`
-- values are deliberately distinct from the post-migration values: the
-- migration re-adds the column, so the result must derive only from the old
-- status and claim ordinal, never from these pre-drop values.
WITH new_user AS (
    INSERT INTO context69.users (login_name, display_name, password_hash)
    VALUES (
        'failure-budget-migration-' || ($1::uuid)::text,
        'Failure Budget Migration Fixture',
        'unused'
    )
    RETURNING id
), new_task AS (
    INSERT INTO context69.tasks (
        id,
        user_id,
        kind,
        status,
        group_path,
        total_count,
        queued_count,
        waiting_count,
        running_count,
        succeeded_count,
        failed_count,
        cancelled_count
    )
    SELECT $1::uuid, id, 'text_batch', 'queued', 'test/failure-budget-migration', 7, 3, 1, 1, 1, 1, 0
    FROM new_user
    RETURNING id
), item_seed (ordinal, status, attempt_count, execution_failure_count, payload) AS (
    VALUES
        (1, 'queued', 5, 1, '{"external_id": "matrix-queued-five"}'::jsonb),
        (2, 'waiting', 5, 2, '{"external_id": "matrix-waiting-five"}'::jsonb),
        (3, 'queued', 4, 3, '{"external_id": "matrix-queued-four"}'::jsonb),
        (4, 'running', 5, 4, '{"external_id": "matrix-running-five"}'::jsonb),
        (5, 'failed', 5, 5, '{"external_id": "matrix-failed-five"}'::jsonb),
        (6, 'succeeded', 5, 0, '{"external_id": "matrix-succeeded-five"}'::jsonb),
        (7, 'queued', 2, 2, '{"external_id": "matrix-history-ordinal"}'::jsonb)
), new_items AS (
    INSERT INTO context69.task_items (
        id,
        task_id,
        ordinal,
        payload,
        status,
        attempt_count,
        execution_failure_count
    )
    SELECT
        gen_random_uuid(),
        new_task.id,
        item_seed.ordinal,
        item_seed.payload,
        item_seed.status,
        item_seed.attempt_count,
        item_seed.execution_failure_count
    FROM item_seed
    CROSS JOIN new_task
    RETURNING id, ordinal
), new_history AS (
    INSERT INTO context69.task_attempts (task_id, item_id, attempt, status, finished_at)
    SELECT
        new_task.id,
        new_items.id,
        history.attempt,
        'failed',
        now() - interval '1 hour'
    FROM new_items
    CROSS JOIN new_task
    CROSS JOIN (VALUES (3), (3), (7)) AS history(attempt)
    WHERE new_items.ordinal = 7
    RETURNING id
)
SELECT new_user.id
FROM new_user
CROSS JOIN (SELECT count(*) AS history_rows FROM new_history) AS history_count;
