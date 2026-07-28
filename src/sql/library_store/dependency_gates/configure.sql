WITH candidate AS (
    SELECT
        dependency_key,
        state,
        last_error,
        configuration_fingerprint,
        CASE
            WHEN $2::BOOLEAN
                THEN state <> 'closed'
                    AND configuration_fingerprint IS DISTINCT FROM $4::TEXT
            ELSE state <> 'open'
        END AS transitioned
    FROM context69.library_dependency_gates
    WHERE dependency_key = $1
    FOR UPDATE
), updated AS (
    UPDATE context69.library_dependency_gates gate
        SET state = CASE
            WHEN NOT $2::BOOLEAN THEN 'open'
            WHEN gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN 'closed'
            ELSE gate.state
        END,
        failure_count = CASE
            WHEN NOT $2::BOOLEAN THEN GREATEST(gate.failure_count, 1)
            WHEN gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN 0
            ELSE gate.failure_count
        END,
        next_probe_at = CASE
            WHEN NOT $2::BOOLEAN THEN NULL
            WHEN gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN NULL
            ELSE gate.next_probe_at
        END,
        last_error = CASE
            WHEN NOT $2::BOOLEAN THEN $3
            WHEN gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN NULL
            ELSE gate.last_error
        END,
        configuration_fingerprint = $4::TEXT,
        probe_lease_token = CASE
            WHEN NOT $2::BOOLEAN THEN NULL
            WHEN gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN NULL
            ELSE gate.probe_lease_token
        END,
        probe_lease_expires_at = CASE
            WHEN NOT $2::BOOLEAN THEN NULL
            WHEN gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN NULL
            ELSE gate.probe_lease_expires_at
        END,
        last_success_at = CASE
            WHEN $2::BOOLEAN
                AND gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN now()
            ELSE gate.last_success_at
        END,
        last_transition_at = CASE
            WHEN NOT $2::BOOLEAN AND gate.state <> 'open' THEN now()
            WHEN $2::BOOLEAN
                AND gate.state <> 'closed'
                AND gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                THEN now()
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
