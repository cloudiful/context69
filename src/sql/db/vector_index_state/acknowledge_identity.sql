UPDATE context69.vector_index_state
SET fingerprint = $1,
    embedding_base_url = $2,
    embedding_model = $3,
    dimensions = $4
WHERE collection_name = $5
  AND fingerprint = $6
  AND dimensions = $7
