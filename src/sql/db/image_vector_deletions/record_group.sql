-- Turn every recorded origin of a deleted group into an exact deletion target,
-- before the group rows disappear.
--
-- $1 group_id.
INSERT INTO context69.image_vector_deletions (
    group_id,
    file_id,
    attempt_id,
    point_id,
    content_sha256,
    runtime_fingerprint,
    endpoint,
    collection_name
)
SELECT o.group_id,
       o.file_id,
       o.attempt_id,
       o.point_id,
       o.content_sha256,
       o.runtime_fingerprint,
       o.endpoint,
       o.collection_name
FROM context69.image_vector_origins o
WHERE o.group_id = $1
ON CONFLICT (endpoint, collection_name, point_id) DO UPDATE
SET group_id = EXCLUDED.group_id,
    file_id = EXCLUDED.file_id,
    content_sha256 = EXCLUDED.content_sha256,
    runtime_fingerprint = EXCLUDED.runtime_fingerprint,
    purged_at = NULL,
    attempts = 0,
    last_error = NULL,
    updated_at = now()
