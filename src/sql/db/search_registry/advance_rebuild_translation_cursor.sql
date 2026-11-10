-- Advance the independent translation-embedding cursor of one owned rebuild.
UPDATE context69.search_registry_epoch
SET rebuild_translation_cursor_document_id = $5::bigint,
    rebuild_translation_cursor_translation_id = $6::uuid,
    rebuild_translation_cursor_chunk_index = $7::integer,
    rebuild_translation_processed_chunks = $8::bigint,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
  AND $7::integer >= 0
  AND $8::bigint >= 0
RETURNING TRUE AS "updated!"
