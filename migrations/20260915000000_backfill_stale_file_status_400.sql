-- Issue #400 file-state cutover: reclaim stale file rows.
--
-- Files only hold terminal states (`succeeded`/`failed`); `processing` is
-- derived from active task_items. Rows stuck in `pending`/`running` (3916 in
-- production) plus legacy `cancelled` rows with no active task_items are
-- backfilled to `failed` with a reclaim marker so the existing retry/rerun
-- channels can revive them. Rows with live tasks are left alone and converge
-- via the terminal item projection (`project_file_status`) and the
-- exhaustion path. Idempotent: a repeat run matches zero rows.
UPDATE context69.library_files file
SET ingest_status = 'failed',
    error_message = 'stale pending/running reclaimed by #400',
    ingested_at = NULL,
    updated_at = now()
WHERE file.ingest_status IN ('pending', 'running', 'cancelled')
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items active
      WHERE active.file_id = file.id
        AND active.status IN ('queued', 'running', 'waiting')
  );
