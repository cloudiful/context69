-- Count Docling poll HTTP reservations in the trailing 30 second window
-- (issue #118 poll-limits). `last_polled_at` is set on reservation and on
-- every poll completion, so the count bounds concurrent poll HTTPs across
-- processes without a new settings knob; the ceiling is the persisted
-- `max_inflight` read by the caller in the same transaction.
SELECT COUNT(*) AS "count!"
FROM context69.task_external_jobs
WHERE provider = $1
  AND last_polled_at > now() - interval '30 seconds'
