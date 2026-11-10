-- The S3 access key stays in the settings row because it is an identifier, not a
-- credential. The secret key is never written here: the store is its only
-- representation, so a settings save cannot leave a plaintext duplicate behind.
--
-- The registry capacity caps are written with the resolved policy: the save
-- resolved them inside this transaction, so a request that did not send a cap
-- keeps the stored one.
INSERT INTO context69.runtime_file_library_settings (
    singleton,
    storage_root,
    max_upload_size_mb,
    trusted_proxy_enabled,
    s3_endpoint,
    s3_region,
    s3_bucket,
    s3_prefix,
    s3_path_style,
    s3_access_key,
    registry_max_ids_total,
    registry_max_ids_per_file,
    registry_max_ids_per_run,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now())
ON CONFLICT (singleton) DO UPDATE
SET storage_root = EXCLUDED.storage_root,
    max_upload_size_mb = EXCLUDED.max_upload_size_mb,
    trusted_proxy_enabled = EXCLUDED.trusted_proxy_enabled,
    s3_endpoint = EXCLUDED.s3_endpoint,
    s3_region = EXCLUDED.s3_region,
    s3_bucket = EXCLUDED.s3_bucket,
    s3_prefix = EXCLUDED.s3_prefix,
    s3_path_style = EXCLUDED.s3_path_style,
    s3_access_key = EXCLUDED.s3_access_key,
    registry_max_ids_total = EXCLUDED.registry_max_ids_total,
    registry_max_ids_per_file = EXCLUDED.registry_max_ids_per_file,
    registry_max_ids_per_run = EXCLUDED.registry_max_ids_per_run,
    updated_at = now()
