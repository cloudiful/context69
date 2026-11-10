-- Generations that may be removed for one repository.
--
-- Successful generations are ranked as a total that includes the active
-- generation, so `retention_count` means all retained snapshots and the default
-- of two keeps the active one plus one previous. The active generation is never
-- a candidate. Successful generations beyond the configured total are
-- candidates; so are failed generations and abandoned building generations
-- older than one hour. A generation that still owes vector cleanup stays a
-- candidate regardless of rank, but a pending retry only becomes eligible once
-- its bounded backoff has elapsed, so a permanently failing source cannot be
-- retried on every pass.
--
-- $1 group_id, $2 repository_key, $3 retention_count.
SELECT candidates.generation_key
FROM (
    SELECT
        g.generation_key,
        g.cleanup_pending,
        g.cleanup_next_retry_at,
        g.status,
        row_number() OVER (
            PARTITION BY g.repository_key
            ORDER BY g.generation_number DESC
        ) AS successful_rank
    FROM context69.git_repository_generations g
    JOIN context69.git_repository_sources s
        ON s.repository_key = g.repository_key
    WHERE s.group_id = $1
      AND g.repository_key = $2
      AND g.status IN ('ready', 'superseded')
) candidates
WHERE candidates.generation_key <> COALESCE(
          (
              SELECT ag.generation_key
              FROM context69.git_repository_active_generations ag
              WHERE ag.repository_key = $2
          ),
          '00000000-0000-0000-0000-000000000000'::uuid
      )
  AND (candidates.successful_rank > $3 OR candidates.cleanup_pending)
  AND (
      NOT candidates.cleanup_pending
      OR candidates.cleanup_next_retry_at IS NULL
      OR candidates.cleanup_next_retry_at <= now()
  )
UNION
SELECT g.generation_key
FROM context69.git_repository_generations g
JOIN context69.git_repository_sources s
    ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
  AND g.status = 'failed'
  AND g.generation_key <> COALESCE(
      (
          SELECT ag.generation_key
          FROM context69.git_repository_active_generations ag
          WHERE ag.repository_key = g.repository_key
      ),
      '00000000-0000-0000-0000-000000000000'::uuid
  )
  AND (
      NOT g.cleanup_pending
      OR g.cleanup_next_retry_at IS NULL
      OR g.cleanup_next_retry_at <= now()
  )
UNION
SELECT g.generation_key
FROM context69.git_repository_generations g
JOIN context69.git_repository_sources s
    ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
  AND g.status = 'building'
  AND g.started_at < now() - interval '1 hour'
  AND g.generation_key <> COALESCE(
      (
          SELECT ag.generation_key
          FROM context69.git_repository_active_generations ag
          WHERE ag.repository_key = g.repository_key
      ),
      '00000000-0000-0000-0000-000000000000'::uuid
  )
  AND (
      NOT g.cleanup_pending
      OR g.cleanup_next_retry_at IS NULL
      OR g.cleanup_next_retry_at <= now()
  )
