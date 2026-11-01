-- Whether a library file already carries another task item's durable ingest
-- history.
--
-- Translation-only registry rows have no durable ingest run, so they never
-- trigger the legacy writer guard. A row here means a replacement (or its
-- retained history) owns this file and the destructive legacy reindex must not
-- run.
SELECT EXISTS (
    SELECT 1
    FROM context69.library_ingest_runs
    WHERE file_id = $1
      AND item_id <> $2
) AS "exists!"
