WITH candidate AS (
    SELECT
        dependency_key,
        state,
        last_error,
        configuration_fingerprint,
        CASE
            WHEN $2::BOOLEAN
                THEN state <> 'closed'
                    AND (
                        configuration_fingerprint IS DISTINCT FROM $4::TEXT
                        -- Issue #176 dirty recovery: stale `configuration:`
                        -- rows that actually carry DNS/connect signals
                        -- without true auth evidence heal even when the
                        -- fingerprint is unchanged (restart with same config).
                        OR (
                            COALESCE(last_error, '') LIKE 'configuration:%'
                            AND (
                                last_error ILIKE '%dns%'
                                OR last_error ILIKE '%resolve%'
                                OR last_error ILIKE '%getaddrinfo%'
                                OR last_error ILIKE '%connect%'
                                OR last_error ILIKE '%timeout%'
                                OR last_error ILIKE '%timed out%'
                                OR last_error ILIKE '%transport%'
                                OR last_error ILIKE '%network%'
                                OR last_error ILIKE '%unreachable%'
                                OR last_error ILIKE '%temporar%'
                            )
                            AND last_error NOT ILIKE '%unauthorized%'
                            AND last_error NOT ILIKE '%forbidden%'
                            AND last_error NOT ILIKE '%authentication%'
                            AND last_error NOT ILIKE '%permission denied%'
                            AND last_error NOT ILIKE '%access denied%'
                            AND last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                        )
                    )
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
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN 'closed'
            ELSE gate.state
        END,
        failure_count = CASE
            WHEN NOT $2::BOOLEAN THEN GREATEST(gate.failure_count, 1)
            WHEN gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN 0
            ELSE gate.failure_count
        END,
        next_probe_at = CASE
            WHEN NOT $2::BOOLEAN THEN NULL
            WHEN gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN NULL
            ELSE gate.next_probe_at
        END,
        last_error = CASE
            WHEN NOT $2::BOOLEAN THEN $3
            WHEN gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN NULL
            ELSE gate.last_error
        END,
        configuration_fingerprint = $4::TEXT,
        probe_lease_token = CASE
            WHEN NOT $2::BOOLEAN THEN NULL
            WHEN gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN NULL
            ELSE gate.probe_lease_token
        END,
        probe_lease_expires_at = CASE
            WHEN NOT $2::BOOLEAN THEN NULL
            WHEN gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN NULL
            ELSE gate.probe_lease_expires_at
        END,
        last_success_at = CASE
            WHEN $2::BOOLEAN
                AND gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
                THEN now()
            ELSE gate.last_success_at
        END,
        last_transition_at = CASE
            WHEN NOT $2::BOOLEAN AND gate.state <> 'open' THEN now()
            WHEN $2::BOOLEAN
                AND gate.state <> 'closed'
                AND (
                    gate.configuration_fingerprint IS DISTINCT FROM $4::TEXT
                    OR (
                        COALESCE(gate.last_error, '') LIKE 'configuration:%'
                        AND (
                            gate.last_error ILIKE '%dns%'
                            OR gate.last_error ILIKE '%resolve%'
                            OR gate.last_error ILIKE '%getaddrinfo%'
                            OR gate.last_error ILIKE '%connect%'
                            OR gate.last_error ILIKE '%timeout%'
                            OR gate.last_error ILIKE '%timed out%'
                            OR gate.last_error ILIKE '%transport%'
                            OR gate.last_error ILIKE '%network%'
                            OR gate.last_error ILIKE '%unreachable%'
                            OR gate.last_error ILIKE '%temporar%'
                        )
                        AND gate.last_error NOT ILIKE '%unauthorized%'
                        AND gate.last_error NOT ILIKE '%forbidden%'
                        AND gate.last_error NOT ILIKE '%authentication%'
                        AND gate.last_error NOT ILIKE '%permission denied%'
                        AND gate.last_error NOT ILIKE '%access denied%'
                        AND gate.last_error !~* '(^|[^A-Za-z0-9])(401|403)([^A-Za-z0-9]|$)'
                    )
                )
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
