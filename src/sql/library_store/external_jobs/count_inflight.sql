-- Count Docling remote jobs holding a slot (issue #118, P1 fix issue #446).
--
-- `pending`/`running` are remote non-terminal states that occupy Docling Serve,
-- but only while their `deadline_at` has not passed. Expired rows
-- (`deadline_at <= now()`) no longer hold a slot so a wedged remote job
-- cannot block new submissions; the dispatcher maintenance recycles them to
-- `timed_out` on the recovery tick. `NULL` deadlines are conservative and
-- still count because they never expire.
-- Fresh `submitting` rows (last 10 minutes) are reservations between the
-- atomic admission insert and the remote POST; they must count so concurrent
-- submitters cannot both slip through. Older `submitting` rows are uncertain
-- historical leftovers (phase 4 handles them) and are deliberately ignored
-- so they cannot permanently wedge admission.
SELECT COUNT(*) AS "count!"
FROM context69.task_external_jobs
WHERE provider = $1
  AND (
    (
      status IN ('pending', 'running')
      AND (deadline_at IS NULL OR deadline_at > now())
    )
    OR (status = 'submitting' AND submitted_at > now() - interval '10 minutes')
  )
