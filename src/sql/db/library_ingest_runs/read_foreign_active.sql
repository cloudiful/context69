-- The current replacement run another task item holds on a library file.
--
-- Only an unresolved current run fences an ordinary replacement. Retired rows
-- are history and a resolved row belongs to a finished switch, so neither may
-- block a new item from replacing the same file.
SELECT item_id
FROM context69.library_ingest_runs
WHERE file_id = $1
  AND item_id <> $2
  AND resolved_at IS NULL
  AND retired_at IS NULL
LIMIT 1
