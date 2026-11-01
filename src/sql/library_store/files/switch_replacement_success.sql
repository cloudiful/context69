UPDATE context69.library_files AS file
SET folder_id = CASE WHEN $13 THEN $6::uuid ELSE file.folder_id END,
    external_id = CASE WHEN $13 THEN $7::text ELSE file.external_id END,
    filename = CASE WHEN $13 THEN $8::text ELSE file.filename END,
    media_type = CASE WHEN $13 THEN $9::text ELSE file.media_type END,
    source_uri = CASE WHEN $13 THEN $10::text ELSE file.source_uri END,
    published_at = CASE WHEN $13 THEN $11::timestamptz ELSE file.published_at END,
    metadata_json = CASE WHEN $13 THEN $12::jsonb ELSE file.metadata_json END,
    storage_object_id = CASE WHEN $4::uuid IS NULL THEN file.storage_object_id ELSE object.id END,
    storage_rel_path = CASE WHEN $4::uuid IS NULL THEN file.storage_rel_path ELSE object.object_key END,
    sha256 = CASE WHEN $4::uuid IS NULL THEN file.sha256 ELSE object.sha256 END,
    size_bytes = CASE WHEN $4::uuid IS NULL THEN file.size_bytes ELSE object.size_bytes END,
    source_released_at = CASE WHEN $4::uuid IS NULL THEN file.source_released_at ELSE NULL END,
    ingest_status = 'succeeded', error_message = NULL, ingested_at = now(), updated_at = now()
FROM (SELECT $4::uuid AS staged_id) AS input
LEFT JOIN context69.library_storage_objects AS object ON object.id = input.staged_id
WHERE file.id = $1 AND file.group_id = $2 AND file.updated_at = $3
  AND EXISTS (
      SELECT 1 FROM context69.task_items AS item
      WHERE item.id = $5 AND item.file_id = file.id
        AND ($4::uuid IS NULL OR item.input_storage_object_id = object.id)
  )
  AND ($4::uuid IS NULL OR object.group_id = file.group_id)
RETURNING file.id AS "id!"
