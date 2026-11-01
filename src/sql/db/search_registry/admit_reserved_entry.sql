INSERT INTO context69.search_registry_entries (
    run_id, file_id, document_ids, staged_ids, retired_ids,
    baseline_record_hash, staged_record_hash, generation, phase, reserved_ids,
    embedding_fingerprint, collection_name
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
