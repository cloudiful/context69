-- Remove only the column the immutable migration under test adds, so the exact
-- migration SQL has to recreate it and backfill the fixture rows. No CASCADE:
-- an unexpected dependent object must fail the test instead of being swept.
ALTER TABLE context69.task_items
    DROP COLUMN execution_failure_count;
