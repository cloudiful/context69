-- One task item with the library segment that executes it (issue 1020 P2).
--
-- The segment of a new item is produced in the same transaction as the item
-- itself, so an item is never visible without the segment that runs it and a
-- segment is never visible without its item.
INSERT INTO context69.task_items (
    id,
    task_id,
    ordinal,
    payload,
    stage,
    file_id,
    input_storage_object_id,
    execution_task_id,
    execution_generation
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
