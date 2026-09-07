WITH candidate AS (
    SELECT
        dependency_key,
        (state <> 'open') AS transitioned
    FROM context69.library_dependency_gates
    WHERE dependency_key = $1
      AND (
          state = 'closed'
          OR probe_lease_token = $2
          -- Issue #176: transient `open` gates refresh backoff on new
          -- failures (DNS/connect with exponential backoff). True
          -- `configuration:` stays pinned; dirty misclassified rows heal by
          -- overwriting the stale prefix with the fresh transient error.
          OR (
              state = 'open'
              AND (
                  COALESCE(last_error, '') NOT LIKE 'configuration:%'
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
          )
      )
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
