-- Turn one exact recorded origin into a deletion target, used when a write is
-- abandoned after registration. It never touches another revision of the file.
--
-- $1 group_id, $2 file_id, $3 attempt_id, $4 point_id.
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
  AND o.file_id = $2
  AND o.attempt_id = $3
  AND o.point_id = $4
ON CONFLICT (endpoint, collection_name, point_id) DO UPDATE
SET group_id = EXCLUDED.group_id,
    file_id = EXCLUDED.file_id,
    content_sha256 = EXCLUDED.content_sha256,
    runtime_fingerprint = EXCLUDED.runtime_fingerprint,
    purged_at = NULL,
    attempts = 0,
    last_error = NULL,
    updated_at = now()
