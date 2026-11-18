-- The user-scoped history rows of one view at the start of a bulk clear.
--
-- `$1` is the calling user and `$2` the view. The scope is exactly the scope
-- of the clear: completed (untrashed succeeded) or trash (trashed terminal).
-- Any other view matches nothing.
--
-- The rows are read without a lock. This is the snapshot of the clear: the
-- items of exactly these rows are locked next, and only these ids are
-- re-checked under a lock afterwards, so a task that becomes eligible in
-- between stays outside this transaction.
SELECT task.id
FROM context69.tasks task
WHERE task.user_id = $1
  AND (
    ($2::text = 'completed'
      AND task.deleted_at IS NULL
      AND task.status = 'succeeded')
    OR ($2::text = 'trash'
      AND task.deleted_at IS NOT NULL
      AND task.status IN ('succeeded', 'failed', 'cancelled'))
  )
ORDER BY task.id
