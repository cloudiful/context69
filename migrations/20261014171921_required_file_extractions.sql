-- Required extraction policy flag for issue #89 P3.
--
-- Explicitly requested extraction must succeed: the task-pipeline gate reads
-- this dedicated flag (never caller-owned parameters JSON) to decide whether
-- a failed/skipped/empty extraction blocks the item. Legacy and omitted
-- directives are required (DEFAULT TRUE); only an explicit required=false
-- opts out. The default alone creates no directive: a NULL template key
-- still means no extraction work.

ALTER TABLE context69.library_files
    ADD COLUMN extraction_required BOOLEAN NOT NULL DEFAULT TRUE;
