WITH candidate AS (
    SELECT
        dependency_key,
        (state <> 'open') AS transitioned
    FROM context69.library_dependency_gates
    WHERE dependency_key = $1
      AND (state = 'closed' OR probe_lease_token = $2)
    FOR UPDATE
), updated AS (
    UPDATE context69.library_dependency_gates gate
    SET state = 'open',
        failure_count = LEAST(gate.failure_count + 1, 31),
        next_probe_at = now() + LEAST(
            INTERVAL '10 minutes',
            (INTERVAL '30 seconds' * power(2::DOUBLE PRECISION, LEAST(gate.failure_count, 10)))
        ),
        last_error = $3,
        probe_lease_token = NULL,
        probe_lease_expires_at = NULL,
        last_transition_at = CASE
            WHEN gate.state <> 'open' THEN now()
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
