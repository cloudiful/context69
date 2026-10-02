-- Creates one group-owned provider connection.
--
-- This is the create-only path: unlike the broad upsert it has no conflict
-- clause, so a duplicate (group_id, connection_key) raises the unique violation
-- the caller maps to a bounded 409 instead of silently overwriting the row or
-- re-enabling a connection an operator had disabled. `disabled_at` is absent
-- from both the column list and the returned defaults, so a new row is enabled.
-- `app_private_key_secret_key` is projected but deliberately never inserted: it
-- is a separate purpose and only its own narrow reference statement may move
-- it, exactly as the broad upsert already does.
WITH inserted AS (
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
    inserted.group_id,
    inserted.connection_key,
    inserted.provider_kind,
    inserted.connection_mode,
    inserted.display_name,
    inserted.base_url,
    inserted.credential_secret_key,
    inserted.webhook_secret_key,
    inserted.app_private_key_secret_key,
    inserted.disabled_at,
    inserted.created_at,
    inserted.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility
FROM inserted
JOIN context69.groups g ON g.id = inserted.group_id
