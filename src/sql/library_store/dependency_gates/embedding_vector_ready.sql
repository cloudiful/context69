SELECT EXISTS (
    SELECT 1
    FROM context69.library_dependency_gates
    WHERE dependency_key = 'embedding_vector'
      AND state = 'closed'
) AS "ready!: bool"
