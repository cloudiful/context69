-- Record when a pending tombstone was last attempted for physical purge.
--
-- The scheduled cleanup claims actionable (known-origin) rows in least-recent
-- attempt order, so a failed or unreachable origin cannot pin the head of the
-- bounded batch forever. Unknown origins stay NULL and remain reportable in the
-- raw pending list; they are never guessed from current settings.

ALTER TABLE context69.library_deleted_entities
    ADD COLUMN IF NOT EXISTS last_purge_attempt_at TIMESTAMPTZ;

COMMENT ON COLUMN context69.library_deleted_entities.last_purge_attempt_at IS
    'When the scheduled purge last attempted this known-origin tombstone; NULL means never attempted or an unknown origin.';

CREATE INDEX IF NOT EXISTS library_deleted_entities_purge_attempt_idx
    ON context69.library_deleted_entities (
        COALESCE(last_purge_attempt_at, deleted_at),
        id
    )
    WHERE purged_at IS NULL
      AND target_endpoint IS NOT NULL
      AND target_collection IS NOT NULL;
