UPDATE context69.image_vector_deletions
SET attempts = attempts + 1,
    last_error = $2,
    updated_at = now()
WHERE id = $1 AND purged_at IS NULL
