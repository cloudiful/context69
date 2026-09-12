-- Due-order claim of cleanup intents. `FOR UPDATE SKIP LOCKED` lets workers
-- drain the queue without blocking each other; `next_attempt_at` ordering
-- keeps a repeatedly failing intent from starving newer work.
SELECT id,
       object_id,
       group_id,
       sha256,
       object_key,
       storage_backend,
       attempts,
       next_attempt_at,
       last_error,
       created_at
FROM context69.library_storage_object_cleanup
WHERE completed_at IS NULL
  AND next_attempt_at <= now()
ORDER BY next_attempt_at, id
LIMIT $1
FOR UPDATE SKIP LOCKED
