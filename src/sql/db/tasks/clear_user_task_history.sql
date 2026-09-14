-- User-scoped bulk clear for task history (issue 391 Task 2).
--
-- A single DELETE statement per view, strictly scoped to the calling user:
--   completed: current user, untrashed, succeeded only.
--   trash: current user, trashed, terminal only (succeeded/failed/cancelled).
--
-- Active tasks, other users' rows, and non-matching views delete nothing.
-- Only context69.tasks rows are deleted; task_items/task_attempts/
-- task_external_jobs cascade via ON DELETE CASCADE while library files,
-- documents, vectors, and S3 objects are owned elsewhere and untouched.
DELETE FROM context69.tasks
WHERE user_id = $1
  AND (
    ($2::text = 'completed'
      AND deleted_at IS NULL
      AND status = 'succeeded')
    OR ($2::text = 'trash'
      AND deleted_at IS NOT NULL
      AND status IN ('succeeded', 'failed', 'cancelled'))
  )
