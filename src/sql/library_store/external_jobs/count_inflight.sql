-- Count Docling remote jobs holding a slot (issue #118).
--
-- `pending`/`running` are remote non-terminal states that occupy Docling Serve.
-- Fresh `submitting` rows (last 10 minutes) are reservations between the
-- atomic admission insert and the remote POST; they must count so concurrent
-- submitters cannot both slip through. Older `submitting` rows are uncertain
-- historical leftovers (phase 4 handles them) and are deliberately ignored
-- so they cannot permanently wedge admission.
SELECT COUNT(*) AS "count!"
FROM context69.task_external_jobs
WHERE provider = $1
  AND (
    status IN ('pending', 'running')
    OR (status = 'submitting' AND submitted_at > now() - interval '10 minutes')
  )
