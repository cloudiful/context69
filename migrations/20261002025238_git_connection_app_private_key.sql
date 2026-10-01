-- GitHub App private-key reference for provider connections (issue #681 work
-- unit 4A3b-5).
--
-- A tracked or private Git connection authenticates with one of two distinct
-- reversible secrets: a personal access token, or a GitHub App private key.
-- They have different lifecycles — an installation is rotated with its app, a
-- token is reissued by hand — and a connection may carry both during a
-- migration. Purpose separation therefore gives the App key its own store
-- purpose and its own nullable reference here, leaving
-- `credential_secret_key` owned by the token alone. A webhook signing secret
-- belongs to a registration in `git_webhook_registrations`, not to a
-- connection, so this column never carries one.
--
-- Only a reference is stored, never key material: the value is sealed by the
-- shared encrypted store and this column names the row that owns it.
--
-- Additive and rollback-safe. The column is NULL until a writer seals an App
-- key through the store, existing rows are untouched, and a release that
-- predates this column keeps serving the same records. It is nullable with
-- `ON DELETE SET NULL` like the existing secret references, so clearing a
-- store row detaches the reference instead of failing on a dangling foreign
-- key, and a connection with no App key is a normal state rather than an
-- error.

ALTER TABLE context69.git_provider_connections
    ADD COLUMN app_private_key_secret_key TEXT
        REFERENCES context69.internal_secrets(key) ON DELETE SET NULL;
