-- Sources with at least one generation whose vector cleanup is due.
--
-- Maintenance is independent of refresh eligibility: manual-only and disabled
-- sources are included because retention cleanup must finish without waiting
-- for another index. A source already marked for deletion is excluded; the
-- deletion retry owns every generation of that source.
--
-- Two kinds of generation are due. A pending retry belongs to an ordinary
-- retention failure. An abandoned `building` generation older than one hour has
-- no pending flag when its process was cancelled or died before recording one,
-- so it is due on age alone. Its provisional destination, if any, must still be
-- cleaned before the row disappears.
--
-- Fairness: only due generations are eligible, and sources are ordered by their
-- earliest due retry. A source that has never been attempted (or whose bounded
-- backoff elapsed) is ordered before a source that keeps failing, so a bounded
-- batch cannot permanently starve a later recoverable source. A source whose
-- lifetime guard was held has a short scheduler-only deferral and is skipped
-- until it elapses, so one long index run cannot fill every batch.
--
-- $1 limit.
SELECT
    s.group_id,
    s.repository_key,
    s.retention_count
FROM context69.git_repository_sources s
WHERE s.deletion_requested_at IS NULL
  AND (s.cleanup_deferred_until IS NULL OR s.cleanup_deferred_until <= now())
  AND EXISTS (
      SELECT 1
      FROM context69.git_repository_generations g
      WHERE g.repository_key = s.repository_key
        AND (
            (
                g.cleanup_pending
                AND (g.cleanup_next_retry_at IS NULL OR g.cleanup_next_retry_at <= now())
            )
            OR (
                g.status = 'building'
                AND g.started_at < now() - interval '1 hour'
                AND NOT g.cleanup_pending
                AND (g.cleanup_next_retry_at IS NULL OR g.cleanup_next_retry_at <= now())
            )
        )
  )
ORDER BY
    (
        SELECT MIN(COALESCE(g.cleanup_next_retry_at, to_timestamp(0)))
        FROM context69.git_repository_generations g
        WHERE g.repository_key = s.repository_key
          AND (
              (
                  g.cleanup_pending
                  AND (g.cleanup_next_retry_at IS NULL OR g.cleanup_next_retry_at <= now())
              )
              OR (
                  g.status = 'building'
                  AND g.started_at < now() - interval '1 hour'
                  AND NOT g.cleanup_pending
                  AND (g.cleanup_next_retry_at IS NULL OR g.cleanup_next_retry_at <= now())
              )
          )
    ),
    (
        SELECT MIN(g.cleanup_attempts)
        FROM context69.git_repository_generations g
        WHERE g.repository_key = s.repository_key
          AND g.cleanup_pending
    ),
    s.repository_key
LIMIT $1
