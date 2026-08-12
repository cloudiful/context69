ALTER TABLE context69.library_storage_objects
    ADD COLUMN IF NOT EXISTS staging_lease_until TIMESTAMPTZ;

ALTER TABLE context69.task_items
    ADD COLUMN IF NOT EXISTS input_storage_object_id UUID
        REFERENCES context69.library_storage_objects(id) ON DELETE RESTRICT;

CREATE INDEX IF NOT EXISTS idx_task_items_input_storage_object_id
    ON context69.task_items (input_storage_object_id)
    WHERE input_storage_object_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_library_storage_objects_staging_lease
    ON context69.library_storage_objects (staging_lease_until)
    WHERE staging_lease_until IS NOT NULL;
