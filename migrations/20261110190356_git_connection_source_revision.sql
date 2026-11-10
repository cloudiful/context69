-- Invalidate the acquisition revision of every source that uses a provider
-- connection when that connection's provider identity, origin, credential
-- references, or enabled state changes.
--
-- A run captures the source revision before it constructs the provider. A
-- connection attach already changes `connection_key` and bumps the source
-- revision; this trigger covers the other meaningful connection edits so a run
-- that paused on the old credential or origin can never begin or publish under
-- the newer revision.

CREATE OR REPLACE FUNCTION context69.bump_git_sources_for_connection_revision()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.provider_kind IS DISTINCT FROM NEW.provider_kind
        OR OLD.connection_mode IS DISTINCT FROM NEW.connection_mode
        OR OLD.base_url IS DISTINCT FROM NEW.base_url
        OR OLD.credential_secret_key IS DISTINCT FROM NEW.credential_secret_key
        OR OLD.webhook_secret_key IS DISTINCT FROM NEW.webhook_secret_key
        OR OLD.app_private_key_secret_key IS DISTINCT FROM NEW.app_private_key_secret_key
        OR OLD.disabled_at IS DISTINCT FROM NEW.disabled_at
    THEN
        UPDATE context69.git_repository_sources
        SET config_revision = config_revision + 1
        WHERE group_id = NEW.group_id
          AND connection_key = NEW.connection_key;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_git_provider_connections_source_revision
    AFTER UPDATE ON context69.git_provider_connections
    FOR EACH ROW
    EXECUTE FUNCTION context69.bump_git_sources_for_connection_revision();
