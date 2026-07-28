SELECT EXISTS (
    SELECT 1
    FROM context69.library_dependency_gates
    WHERE dependency_key = 'embedding_vector'
      AND (
          state = 'closed'
          OR (state = 'half_open' AND probe_lease_token = $1)
      )
) AS "ready!: bool";
