-- Obsolete plaintext credential columns (issue #681 work unit 4A3b-7).
--
-- Every reversible runtime credential in scope is sealed in
-- `context69.internal_secrets` under a typed purpose, and every consumer reads
-- it from there. These five columns are the plaintext duplicates the additive
-- phases kept for rollback; the manual backfill has already sealed every value
-- they held, so nothing is lost by removing them.
--
-- This migration drops columns and nothing else. It seals nothing, clears
-- nothing, copies nothing, and re-interprets no existing value: whatever the
-- store already holds is what the application will keep serving, and a value
-- that was never sealed is gone with its column. That is why it may only be
-- applied after the backfill has reported every category sealed and every
-- source connection referenced.
--
-- `context69.translation_provider_settings.api_key` is deliberately absent: it
-- still holds the `deepl` and `libretranslate` credentials, which no secret
-- purpose owns. The shared `llm` row no longer writes it, and this migration
-- leaves the column and its two remaining owners untouched.
--
-- Also dropped, with its column, is the non-blank check on a source connection's
-- database URL. PostgreSQL derived the constraint name from that single column,
-- so it is addressed by the name the original `CREATE TABLE` produced. The
-- connection's stable identity, its `internal_secrets` reference, and its name
-- are the whole record now.

ALTER TABLE context69.runtime_embedding_settings
    DROP COLUMN api_key;

ALTER TABLE context69.search_settings
    DROP COLUMN api_key;

ALTER TABLE context69.docling_settings
    DROP COLUMN api_key;

ALTER TABLE context69.runtime_file_library_settings
    DROP COLUMN s3_secret_key;

ALTER TABLE context69.runtime_source_connections
    DROP CONSTRAINT runtime_source_connections_database_url_check,
    DROP COLUMN database_url;
