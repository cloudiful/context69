WITH candidate AS (
    SELECT dependency_key
    FROM context69.library_dependency_gates
    WHERE dependency_key = $1
      AND state = 'half_open'
      AND probe_lease_token = $2
    FOR UPDATE
), updated AS (
    UPDATE context69.library_dependency_gates gate
    SET state = 'closed',
        failure_count = 0,
        next_probe_at = NULL,
        last_error = NULL,
        probe_lease_token = NULL,
        probe_lease_expires_at = NULL,
        last_transition_at = now(),
        updated_at = now()
    FROM candidate
    WHERE gate.dependency_key = candidate.dependency_key
    RETURNING gate.dependency_key, gate.state
)
SELECT updated.dependency_key AS "dependency_key!",
       updated.state AS "state!",
       TRUE AS "transitioned!"
FROM updated;
