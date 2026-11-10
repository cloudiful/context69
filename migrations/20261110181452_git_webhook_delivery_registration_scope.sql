-- Webhook delivery idempotency scoped to the actual registration (issue #28
-- phase P11).
--
-- A provider delivery id is only unique within the registration that received
-- it, so the global `delivery_id` primary key is replaced by the registration
-- identity already modelled in this schema: provider, provider-issued hook id,
-- and the owning repository source. The applied table is not rewritten; this
-- migration only adds the missing hook identity, backfills it from the current
-- registration, and introduces the scoped unique key the record/update
-- statements use.

ALTER TABLE context69.git_webhook_deliveries
    ADD COLUMN external_hook_id TEXT NOT NULL DEFAULT '';

UPDATE context69.git_webhook_deliveries d
SET external_hook_id = r.external_hook_id
FROM context69.git_webhook_registrations r
WHERE d.repository_key = r.repository_key
  AND d.provider_kind = r.provider_kind
  AND d.external_hook_id = '';

ALTER TABLE context69.git_webhook_deliveries
    DROP CONSTRAINT git_webhook_deliveries_pkey;

CREATE UNIQUE INDEX IF NOT EXISTS uq_git_webhook_deliveries_registration
    ON context69.git_webhook_deliveries (
        provider_kind,
        external_hook_id,
        repository_key,
        delivery_id
    );
