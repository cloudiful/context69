SELECT
    dependency_key,
    state,
    failure_count,
    next_probe_at,
    probe_lease_expires_at,
    last_error,
    probe_lease_token,
    last_transition_at,
    last_success_at
FROM context69.library_dependency_gates
ORDER BY dependency_key
