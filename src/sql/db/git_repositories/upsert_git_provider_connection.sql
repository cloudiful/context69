-- Registers or updates a connection's non-secret metadata and its legacy
-- credential/webhook references.
--
-- `app_private_key_secret_key` is projected but deliberately absent from both
-- the column list and the conflict clause: it is a separate secret with its own
-- purpose, and only the narrow reference statement may move it. Leaving it out
-- of the update is what keeps a metadata save from clearing a reference a
-- writer has already established.
WITH upserted AS (
    INSERT INTO context69.git_provider_connections (
        group_id,
        connection_key,
        provider_kind,
        connection_mode,
        display_name,
        base_url,
        credential_secret_key,
        webhook_secret_key
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    ON CONFLICT (group_id, connection_key) DO UPDATE
    SET provider_kind = EXCLUDED.provider_kind,
        connection_mode = EXCLUDED.connection_mode,
        display_name = EXCLUDED.display_name,
        base_url = EXCLUDED.base_url,
        credential_secret_key = EXCLUDED.credential_secret_key,
        webhook_secret_key = EXCLUDED.webhook_secret_key,
        disabled_at = NULL,
        updated_at = now()
    RETURNING
        group_id,
        connection_key,
        provider_kind,
        connection_mode,
        display_name,
        base_url,
        credential_secret_key,
        webhook_secret_key,
        app_private_key_secret_key,
        disabled_at,
        created_at,
        updated_at
)
SELECT
    upserted.group_id,
    upserted.connection_key,
    upserted.provider_kind,
    upserted.connection_mode,
    upserted.display_name,
    upserted.base_url,
    upserted.credential_secret_key,
    upserted.webhook_secret_key,
    upserted.app_private_key_secret_key,
    upserted.disabled_at,
    upserted.created_at,
    upserted.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility
FROM upserted
JOIN context69.groups g ON g.id = upserted.group_id
