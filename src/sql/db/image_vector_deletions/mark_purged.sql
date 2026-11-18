UPDATE context69.image_vector_deletions
SET purged_at = now(),
    last_error = NULL,
    updated_at = now()
WHERE id = $1 AND purged_at IS NULL
