-- Register the exact physical point one publish attempt is about to produce.
--
-- The attempt id is authoritative: identical file/revision/runtime/target
-- writes from two attempts are distinct rows, and only the owning attempt can
-- acknowledge or discard its own origin. The origin is committed before the
-- Qdrant write and marked active only after the write acknowledges.
--
-- $1 attempt_id, $2 group_id, $3 file_id, $4 content_sha256,
-- $5 runtime_fingerprint, $6 endpoint, $7 collection_name, $8 point_id.
INSERT INTO context69.image_vector_origins (
    attempt_id,
    group_id,
    file_id,
    content_sha256,
    runtime_fingerprint,
    endpoint,
    collection_name,
    point_id,
    state,
    attempt_started_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'writing', now())
ON CONFLICT (attempt_id) DO UPDATE
SET point_id = EXCLUDED.point_id,
    state = 'writing',
    updated_at = now()
