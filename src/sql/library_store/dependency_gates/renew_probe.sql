UPDATE context69.library_dependency_gates
SET probe_lease_expires_at = now() + ($3::BIGINT * INTERVAL '1 second'),
    updated_at = now()
WHERE dependency_key = $1
  AND state = 'half_open'
  AND probe_lease_token = $2
RETURNING dependency_key AS "dependency_key!"
