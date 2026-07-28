SELECT
    dependency_key,
    state,
    failure_count,
    next_probe_at,
    last_error,
    configuration_fingerprint,
    probe_lease_token,
    probe_lease_expires_at,
    last_transition_at,
    last_success_at,
    updated_at
FROM context69.library_dependency_gates
ORDER BY dependency_key
