-- Hydrate image search candidates against the exact current origin and file.
--
-- A point is returned only while its origin is active for this group, target
-- and runtime fingerprint, and the file row still exists in the same group
-- with the origin's exact content revision. Stale, deleted and uncertain
-- candidates therefore disappear before any response is built.
--
-- $1 group_id, $2 endpoint, $3 collection_name, $4 runtime_fingerprint,
-- $5 point_ids.
SELECT o.point_id,
       o.file_id,
       f.external_id,
       f.filename,
       f.media_type,
       f.sha256
FROM context69.image_vector_origins o
INNER JOIN context69.library_files f
        ON f.id = o.file_id
       AND f.group_id = o.group_id
       AND f.sha256 = o.content_sha256
WHERE o.group_id = $1
  AND o.state = 'active'
  AND o.endpoint = $2
  AND o.collection_name = $3
  AND o.runtime_fingerprint = $4
  AND o.point_id = ANY($5)
