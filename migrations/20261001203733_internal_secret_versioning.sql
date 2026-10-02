-- Version and purpose metadata for the application-owned encrypted secret
-- store (issue #681 work unit 4A3a).
--
-- `context69.internal_secrets` already held one row per secret with a plaintext
-- `value`. The unified store needs two things before any value can be sealed:
-- a marker that says which representation a row carries, and the owning purpose
-- the ciphertext is bound to. Both are added here as columns, and nothing else
-- changes.
--
-- Additive only. No row is encrypted, cleared, or re-interpreted, and no
-- existing column is dropped or renamed, so a deployment that applies this
-- migration keeps resolving every existing secret exactly as before:
--
--   * `ciphertext_version = 0` means the row still holds the legacy plaintext
--     representation. It is the transition marker the store reads.
--   * `purpose IS NULL` states the same thing in the purpose domain: a row with
--     no owner has not been claimed by a typed accessor yet. The check
--     constraint ties the two markers together, so a row can never claim to be
--     sealed for an owner it does not have.
--   * `key_version = 0` is the master-key version of a legacy row. Sealed rows
--     carry the version their master key was registered under, so a rotated key
--     is detected instead of silently mis-decrypted.
--   * `updated_at` stays NULL on rows this migration does not touch, so the
--     column records when a value was actually written through the store and
--     never claims a write that did not happen.

ALTER TABLE context69.internal_secrets
    ADD COLUMN purpose TEXT,
    ADD COLUMN key_version INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN ciphertext_version INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN updated_at TIMESTAMPTZ;

ALTER TABLE context69.internal_secrets
    ADD CONSTRAINT chk_internal_secrets_ciphertext_metadata
        CHECK (
            key_version >= 0
            AND ciphertext_version >= 0
            AND ((ciphertext_version = 0) = (purpose IS NULL))
        );

-- The transition needs to enumerate the keys whose stored bytes are still
-- legacy plaintext. A partial index keeps that enumeration off a sequential
-- scan once most rows are sealed, and it indexes no value column.
CREATE INDEX internal_secrets_unversioned_idx
    ON context69.internal_secrets (key)
    WHERE ciphertext_version = 0;
