-- Lock the exact current revision of one image file for the short write+ack
-- phase. A deleted or replaced row is observed before the Qdrant call.
--
-- $1 file_id, $2 group_id.
SELECT id, group_id, sha256, media_type, filename, external_id
FROM context69.library_files
WHERE id = $1 AND group_id = $2
FOR UPDATE
