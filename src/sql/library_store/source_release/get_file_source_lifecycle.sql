SELECT id,
       group_id,
       filename,
       external_id,
       ingest_status,
       storage_rel_path,
       storage_object_id,
       sha256,
       size_bytes,
       delete_source_after_processing,
       source_released_at
FROM context69.library_files
WHERE id = $1
