-- Durable old-key record for the later cleanup phase. Committed in the same
-- transaction as the library_files reference update so a migrated reference
-- never exists without its cleanup record. Re-runs are idempotent.
INSERT INTO context69.library_legacy_object_cleanup (
    group_id,
    file_id,
    old_key,
    old_storage_backend,
    cleanup_eligible_at
)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (file_id, old_key) DO NOTHING
