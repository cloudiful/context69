-- Enqueue one replaced collection with its proven pinned endpoint.
--
-- The unique key makes an identical enqueue idempotent: a name that is already
-- pending is never duplicated, and its existing due time is kept.
INSERT INTO context69.vector_collection_retirement (
    endpoint,
    collection_name,
    next_attempt_at
)
VALUES ($1, $2, NOW() + make_interval(secs => ($3)::int))
ON CONFLICT (endpoint, collection_name) DO NOTHING
