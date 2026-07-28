WITH candidate AS (
    SELECT
        dependency_key,
        (state = 'open') AS transitioned
    FROM context69.library_dependency_gates
    WHERE dependency_key = $1
      AND COALESCE(last_error, '') NOT LIKE 'configuration:%'
      AND (
          (state = 'open' AND (next_probe_at IS NULL OR next_probe_at <= now()))
          OR (state = 'half_open' AND (probe_lease_expires_at IS NULL OR probe_lease_expires_at <= now()))
      )
    FOR UPDATE
), updated AS (
    UPDATE context69.library_dependency_gates gate
    SET state = 'half_open',
        probe_lease_token = $2,
        probe_lease_expires_at = now() + ($3::BIGINT * INTERVAL '1 second'),
        last_transition_at = CASE
            WHEN gate.state = 'open' THEN now()
            ELSE gate.last_transition_at
        END,
        updated_at = now()
    FROM candidate
    WHERE gate.dependency_key = candidate.dependency_key
    RETURNING gate.dependency_key, gate.state
)
SELECT updated.dependency_key AS "dependency_key!",
       updated.state AS "state!",
       candidate.transitioned AS "transitioned!"
FROM updated
JOIN candidate USING (dependency_key)
