-- Releases the database-owned lock of the startup handover (issue 1100 P1).
--
-- The release runs on the same session that took the lock, so the handover
-- gives the lock back before its connection returns to the pool. A session that
-- ends without this statement releases the lock as well, which is what makes a
-- cancelled handover safe.
SELECT pg_advisory_unlock(hashtextextended('context69.task_handover', 0)) AS "released!"
