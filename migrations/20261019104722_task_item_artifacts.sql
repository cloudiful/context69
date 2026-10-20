-- Narrow internal references to extracted task-item artifacts (issue #975 P1).
--
-- A row keeps only the item -> (kind, object) identity. Byte size, digest,
-- group, backend and object key stay authoritative in
-- context69.library_storage_objects; this relation never duplicates them.
--
-- Deleting an artifact row, or retargeting it to another object, records one
-- durable source-object cleanup intent for the old object in the existing
-- outbox. Cascade deletes from task_items fire the same row trigger, and the
-- intent commits or rolls back with the reference mutation. A still-staged
-- object keeps its 24-hour staging lease and is released by the existing
-- staging sweep. When the owning group is itself being deleted, the group
-- cascade removes the object and the outbox row through their own foreign
-- keys, so no intent is written for that case.

CREATE TABLE context69.task_item_artifacts (
    item_id UUID NOT NULL
        REFERENCES context69.task_items(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    storage_object_id UUID NOT NULL
        REFERENCES context69.library_storage_objects(id) ON DELETE RESTRICT,
    encoding TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, kind),
    CHECK (
        (kind = 'download' AND encoding = 'raw-v1')
        OR (kind = 'sections' AND encoding = 'ingest-sections-json-v1')
    )
);

CREATE INDEX idx_task_item_artifacts_storage_object_id
    ON context69.task_item_artifacts (storage_object_id);

CREATE OR REPLACE FUNCTION context69.enqueue_task_item_artifact_release()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'UPDATE'
        AND NEW.storage_object_id IS NOT DISTINCT FROM OLD.storage_object_id
    THEN
        RETURN NEW;
    END IF;

    INSERT INTO context69.library_storage_object_cleanup
        (object_id, group_id, sha256, object_key, storage_backend)
    SELECT object.id, object.group_id, object.sha256, object.object_key,
           object.storage_backend
    FROM context69.library_storage_objects AS object
    JOIN context69.groups AS owner ON owner.id = object.group_id
    WHERE object.id = OLD.storage_object_id
      AND object.staging_lease_until IS NULL
    ON CONFLICT DO NOTHING;

    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$;

CREATE TRIGGER trg_task_item_artifacts_release_old_object
AFTER DELETE ON context69.task_item_artifacts
FOR EACH ROW
EXECUTE FUNCTION context69.enqueue_task_item_artifact_release();

CREATE TRIGGER trg_task_item_artifacts_retarget_old_object
AFTER UPDATE OF storage_object_id ON context69.task_item_artifacts
FOR EACH ROW
EXECUTE FUNCTION context69.enqueue_task_item_artifact_release();
