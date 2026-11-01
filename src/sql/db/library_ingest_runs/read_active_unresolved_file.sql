-- Whether a library file carries an active unresolved ingest run.
--
-- An active run is a current row that was never resolved. A retired row is
-- history and a resolved row belongs to a finished switch, so neither may
-- block a delete. The delete of a file with an active run is refused, so the
-- caller can cancel the run and retry instead of deleting a file that an
-- ingest is still writing.
SELECT EXISTS (
    SELECT 1
    FROM context69.library_ingest_runs
    WHERE file_id = $1
      AND resolved_at IS NULL
      AND retired_at IS NULL
      AND ($2::uuid IS NULL OR item_id <> $2)
) AS "active!"
