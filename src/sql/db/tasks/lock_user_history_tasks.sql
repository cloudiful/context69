-- Lock the still-eligible history rows of one view after their items are
-- locked.
--
-- `$1` is the snapshot id set, `$2` the calling user and `$3` the view. The
-- scope is exactly the scope of the clear and of the snapshot: completed
-- (untrashed succeeded) or trash (trashed terminal). Any other view matches
-- nothing.
--
-- The rows are locked in id order, so the eligibility, the native deletion,
-- the cleanup targets and the reported count are all derived from the locked
-- result and never from the earlier snapshot.
SELECT task.id
FROM context69.tasks task
WHERE task.id = ANY($1::uuid[])
  AND task.user_id = $2
  AND (
    ($3::text = 'completed'
      AND task.deleted_at IS NULL
      AND task.status = 'succeeded')
    OR ($3::text = 'trash'
      AND task.deleted_at IS NOT NULL
      AND task.status IN ('succeeded', 'failed', 'cancelled'))
  )
ORDER BY task.id
FOR UPDATE
