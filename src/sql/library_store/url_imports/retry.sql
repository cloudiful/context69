INSERT INTO context69.library_url_import_jobs (
    id,
    group_id,
    visibility,
    folder_id,
    source_url,
    dedupe_key,
    requested_filename,
    requested_media_type,
    external_id,
    source_uri,
    published_at,
    metadata_json,
    metadata_provided,
    translation_provided,
    translation_source_locale,
    translation_target_locales,
    file_id
)
SELECT
    $3,
    group_id,
    visibility,
    folder_id,
    source_url,
    dedupe_key,
    requested_filename,
    requested_media_type,
    external_id,
    source_uri,
    published_at,
    metadata_json,
    metadata_provided,
    translation_provided,
    translation_source_locale,
    translation_target_locales,
    file_id
FROM context69.library_url_import_jobs
WHERE group_id = $1 AND id = $2 AND status = 'failed'
RETURNING *
