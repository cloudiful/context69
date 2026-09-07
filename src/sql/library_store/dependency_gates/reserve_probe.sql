WITH candidate AS (
    SELECT
        dependency_key,
        (state = 'open') AS transitioned
    FROM context69.library_dependency_gates
    WHERE dependency_key = $1
      AND (
          COALESCE(last_error, '') NOT LIKE 'configuration:%'
          -- Issue #176 dirty recovery: a `configuration:` row that actually
          -- carries DNS/connect/timeout signals without true auth evidence
          -- was a transient misclassified by the old bare-`401` matcher
          -- (UUID `401de82e`). Allow it to probe so success/failure can heal
          -- the prefix and backoff. True 401/403 stays blocked.
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
