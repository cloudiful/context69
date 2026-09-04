-- Existence pre-check for queue-only Docling recovery (issue #118 phase 4).
--
-- The main `queue_docling_recovery.sql` statement returns an all-NULL
-- outer-joined row when the task has no Docling-stage item; the typed macro
-- decoder cannot consume that shape (the same pre-existing quirk immediate
-- recovery has on its `no_docling_item` path), so the service checks
-- existence first and only runs the main statement when a Docling-stage
-- item exists. The main statement remains authoritative for every other
-- boundary: a concurrent mutation between the two statements can only make
-- the main statement re-validate, never corrupt state.
SELECT EXISTS (
    SELECT 1
    FROM context69.tasks
    WHERE id = $1
) AS "task_exists!",
       EXISTS (
    SELECT 1
    FROM context69.task_items
    WHERE task_id = $1
      AND stage IN ('docling', 'docling_poll')
) AS "has_docling_item!";
