SELECT run_id, file_id, document_ids, staged_ids, retired_ids,
       baseline_record_hash, staged_record_hash, generation, phase
FROM context69.search_registry_entries
WHERE run_id = $1
FOR UPDATE
