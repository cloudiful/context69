-- Create or increment the deterministic original reservation of one file.
--
-- The run id is derived from the target and the optional file, so every page
-- of the same build increments one reservation row. An existing row may only
-- be incremented when it still carries the exact producer shape and the same
-- file, target, fingerprint and pinned generation; any other row at that id
-- matches no update and the batch transaction aborts instead of double
-- counting.
INSERT INTO context69.search_registry_entries (
    run_id, file_id, document_ids, staged_ids, retired_ids,
    baseline_record_hash, staged_record_hash, generation, phase, reserved_ids,
    embedding_fingerprint, collection_name
) VALUES (
    $1, $2, '{}', '{}', '{}', NULL, NULL, $3, 'switched', $4, $5, $6
)
ON CONFLICT (run_id) DO UPDATE
SET reserved_ids = context69.search_registry_entries.reserved_ids
        + EXCLUDED.reserved_ids
WHERE context69.search_registry_entries.phase = 'switched'
  AND cardinality(context69.search_registry_entries.staged_ids) = 0
  AND cardinality(context69.search_registry_entries.retired_ids) = 0
  AND cardinality(context69.search_registry_entries.document_ids) = 0
  AND context69.search_registry_entries.file_id
        IS NOT DISTINCT FROM EXCLUDED.file_id
  AND context69.search_registry_entries.generation = EXCLUDED.generation
  AND context69.search_registry_entries.embedding_fingerprint
        = EXCLUDED.embedding_fingerprint
  AND context69.search_registry_entries.collection_name
        = EXCLUDED.collection_name
RETURNING run_id
