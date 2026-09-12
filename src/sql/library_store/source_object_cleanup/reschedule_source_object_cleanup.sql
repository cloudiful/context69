-- Push a failed attempt behind fresh work by doubling the delay up to one hour.
UPDATE context69.library_storage_object_cleanup
SET attempts = attempts + 1,
    next_attempt_at = now() + ($2::bigint * INTERVAL '1 second'),
    last_error = $3,
    updated_at = now()
WHERE id = $1
  AND completed_at IS NULL
