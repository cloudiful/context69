-- The rollback to the savepoint around the chunk insert of one guarded batch
-- (issue 1020 P3C).
--
-- The duplicate chunk ids of an earlier partial ingest are rolled back to the
-- savepoint, so the guarded transaction stays usable and can still commit the
-- rest of the batch.
ROLLBACK TO SAVEPOINT task_chunk_insert
