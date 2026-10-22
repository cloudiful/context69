-- A savepoint around the chunk insert of one guarded batch (issue 1020 P3C).
--
-- The chunk insert is idempotent under deterministic chunk ids, so a batch that
-- an earlier partial ingest already wrote raises a duplicate key. The
-- savepoint keeps that one statement from poisoning the guarded transaction,
-- so the duplicate can be rolled back to the savepoint and the transaction can
-- still commit the rest of the batch.
SAVEPOINT task_chunk_insert
