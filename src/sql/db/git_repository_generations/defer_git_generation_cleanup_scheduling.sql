-- Defers one source's generation-cleanup scheduling without recording a
-- failure (issue #28 phase P13).
--
-- A scheduler pass that found the source's index lifetime guard held could not
-- do any cleanup. Leaving the source eligible would let one long index run fill
-- every bounded batch; a short scheduler-only deferral lets other due sources be
-- selected first and retries this one later. The generation's own cleanup retry
-- state is deliberately untouched: this is not evidence that cleanup failed.
--
-- $1 group_id, $2 repository_key.
UPDATE context69.git_repository_sources
SET cleanup_deferred_until = now() + interval '60 seconds'
WHERE group_id = $1
  AND repository_key = $2
  AND deletion_requested_at IS NULL
