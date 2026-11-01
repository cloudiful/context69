SELECT id, group_id, folder_id, visibility, external_id, source_uri, published_at,
       metadata_json, filename, media_type, size_bytes, sha256, storage_rel_path,
       storage_object_id, source_released_at, updated_at
FROM context69.library_files
WHERE id = $1
FOR UPDATE
