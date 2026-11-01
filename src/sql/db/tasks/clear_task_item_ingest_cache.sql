-- The cached ingest state of one running task item (issue 1138 P22).
--
-- An item that enters the atomic ingest mode must read its own current
-- source, so the sections a previous attempt parsed, the indexing checkpoint
-- of that parse and the pipeline progress are dropped. The lease of the item
-- fences the clear: the fencing token of the library attempt, the status of
-- the item and the deadline of its lease.
--
-- The sections artifact of the item is removed in the same transaction, so a
-- verified reference can never survive the inline copy it replaced. No
-- document, point, version or result is touched.
--
-- `$1` is the item, `$2` the fencing token, `$3` the library task and `$4`
-- the execution generation.
WITH cleared_item AS (
    UPDATE context69.task_items AS item
    SET payload = (item.payload - 'section_payload')
                   - 'indexing_checkpoint'
                   - 'pipeline_progress',
        updated_at = now()
    WHERE item.id = $1
      AND item.lease_token = $2
      AND item.status = 'running'
      AND item.lease_until > clock_timestamp()
      AND item.execution_task_id = $3
      AND item.execution_generation = $4
    RETURNING item.id
), dropped_artifact AS (
    DELETE FROM context69.task_item_artifacts
    WHERE item_id IN (SELECT id FROM cleared_item)
      AND kind = 'sections'
    RETURNING item_id
)
SELECT id FROM cleared_item;
