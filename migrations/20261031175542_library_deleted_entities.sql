-- Entity tombstones in the coherent search snapshot (issue #1138 P24).
--
-- A deleted library file or an explicitly deleted document leaves the search
-- results through its entity identity, not through one registry reservation
-- per point: the search snapshot reads the tombstoned file and document ids
-- and the index filters them by the indexed `library_file_id` and
-- `document_id` payload fields.
--
-- The table carries no foreign key at all. A tombstone must survive the
-- deletion of the group, the file, the task and the document rows it names,
-- and a physical purge of those rows never removes it, so the exclusion can
-- never be lost by a cascading delete.

CREATE TABLE IF NOT EXISTS context69.library_deleted_entities (
    id BIGSERIAL PRIMARY KEY,
    file_id UUID,
    document_id BIGINT,
    group_id BIGINT,
    deleted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    purged_at TIMESTAMPTZ,
    CONSTRAINT chk_library_deleted_entities_target
        CHECK ((file_id IS NOT NULL) <> (document_id IS NOT NULL))
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_deleted_entities_file
    ON context69.library_deleted_entities (file_id)
    WHERE file_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_deleted_entities_document
    ON context69.library_deleted_entities (document_id)
    WHERE document_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_library_deleted_entities_pending
    ON context69.library_deleted_entities (deleted_at, id)
    WHERE purged_at IS NULL;

-- Backstop for the file delete path.
--
-- A supported delete caller records the tombstone explicitly and acquires the
-- registry epoch before the file row. This trigger covers every other
-- delete, including a raw statement that bypasses the service: the tombstone
-- and the two generation bumps happen in the deleting transaction, so the
-- search snapshot never shows a live file whose points are already gone.
--
-- The lock order of the trigger is file (the row being deleted) then epoch,
-- which is the reverse of the supported service order. A delete that races a
-- supported writer can therefore abort on a deadlock instead of silently
-- violating the visibility rule.
CREATE OR REPLACE FUNCTION context69.tombstone_library_file_before_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    inserted_rows INTEGER;
BEGIN
    INSERT INTO context69.library_deleted_entities (file_id, group_id)
    VALUES (OLD.id, OLD.group_id)
    ON CONFLICT (file_id) WHERE file_id IS NOT NULL DO NOTHING;

    GET DIAGNOSTICS inserted_rows = ROW_COUNT;

    IF inserted_rows > 0 THEN
        UPDATE context69.search_registry_epoch
        SET epoch = context69.search_registry_epoch.epoch + 1,
            updated_at = now()
        WHERE id = 1;

        INSERT INTO context69.search_generations (scope, generation, updated_at)
        VALUES ('global', 1, now())
        ON CONFLICT (scope) DO UPDATE
        SET generation = context69.search_generations.generation + 1,
            updated_at = now();
    END IF;

    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS trg_library_files_tombstone_before_delete ON context69.library_files;
CREATE TRIGGER trg_library_files_tombstone_before_delete
    BEFORE DELETE ON context69.library_files
    FOR EACH ROW
    EXECUTE FUNCTION context69.tombstone_library_file_before_delete();

-- Reuse guard for the explicit file id.
--
-- A tombstoned file id is never handed out again: the file is gone, so a new
-- row with the same id would be invisible to every search that still excludes
-- the tombstone.
CREATE OR REPLACE FUNCTION context69.refuse_tombstoned_library_file_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM context69.library_deleted_entities
        WHERE file_id = NEW.id
    ) THEN
        RAISE EXCEPTION 'library file % is deleted and its id cannot be reused', NEW.id
            USING ERRCODE = '23505';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_library_files_refuse_tombstoned_id ON context69.library_files;
CREATE TRIGGER trg_library_files_refuse_tombstoned_id
    BEFORE INSERT ON context69.library_files
    FOR EACH ROW
    EXECUTE FUNCTION context69.refuse_tombstoned_library_file_insert();

-- Reuse guard for the explicit document id.
--
-- The document sequence never repeats a value, so an explicitly inserted
-- document id is the only way a tombstoned id can come back.
CREATE OR REPLACE FUNCTION context69.refuse_tombstoned_document_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM context69.library_deleted_entities
        WHERE document_id = NEW.id
    ) THEN
        RAISE EXCEPTION 'document % is deleted and its id cannot be reused', NEW.id
            USING ERRCODE = '23505';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_documents_refuse_tombstoned_id ON context69.documents;
CREATE TRIGGER trg_documents_refuse_tombstoned_id
    BEFORE INSERT ON context69.documents
    FOR EACH ROW
    EXECUTE FUNCTION context69.refuse_tombstoned_document_insert();
