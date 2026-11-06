-- The durable state of the vector index rebuild control surface.
--
-- One row per vector rebuild task, plus the one active vector index the
-- deployment currently serves.
--
-- The base row keeps the projection readable when the deployment never ran a
-- rebuild: the active index is still reported, and the task columns are null.
-- The active pointer is unique (at most one collection is active), so the
-- active columns are the same on every row.
--
-- The task that owns the durable rebuild barrier is ordered first, so a newer
-- queued rebuild never hides the running or paused owner of the barrier. The
-- owner is read from the epoch row itself, which records it while a rebuild
-- runs or while a failed rebuild still holds the barrier.
--
-- The item is the lowest-ordinal item of the task: a rebuild is submitted
-- with one item, and the parked dependency of a paused rebuild is recorded on
-- it. `item_dependency_key` is what tells a parked rebuild from a running one.
SELECT
    active.collection_name AS active_collection_name,
    active.embedding_model AS active_model,
    active.dimensions AS active_dimensions,
    active.rebuilt_chunks AS active_rebuilt_chunks,
    task.id AS id,
    task.status AS status,
    task.stage AS stage,
    task.waiting_reason AS waiting_reason,
    task.dependency_key AS dependency_key,
    task.error_summary AS error_summary,
    task.created_at AS created_at,
    task.updated_at AS updated_at,
    task.started_at AS started_at,
    task.finished_at AS finished_at,
    task.item_id AS item_id,
    task.item_status AS item_status,
    task.item_dependency_key AS item_dependency_key
FROM (SELECT 1 AS base) base
LEFT JOIN LATERAL (
    SELECT collection_name, embedding_model, dimensions, rebuilt_chunks
    FROM context69.vector_index_state
    WHERE active
    LIMIT 1
) active ON TRUE
LEFT JOIN LATERAL (
    SELECT
        task.id,
        task.status,
        task.stage,
        task.waiting_reason,
        task.dependency_key,
        task.error_summary,
        task.created_at,
        task.updated_at,
        task.started_at,
        task.finished_at,
        item.id AS item_id,
        item.status AS item_status,
        item.dependency_key AS item_dependency_key
    FROM context69.tasks task
    LEFT JOIN LATERAL (
        SELECT id, status, dependency_key
        FROM context69.task_items
        WHERE task_id = task.id
        ORDER BY ordinal
        LIMIT 1
    ) item ON TRUE
    WHERE task.kind = 'vector_rebuild'
      AND task.deleted_at IS NULL
    ORDER BY
        (item.id = (
            SELECT epoch.rebuild_owner_item
            FROM context69.search_registry_epoch epoch
            WHERE epoch.id = 1
        )) DESC NULLS LAST,
        task.created_at DESC,
        task.id DESC
    LIMIT 20
) task ON TRUE
