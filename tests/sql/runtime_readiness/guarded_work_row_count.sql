-- Startup resumes, migrates, or cleans up every table summed here, so a
-- scratch database must hold none of this work before an application boot.
SELECT
    (SELECT count(*) FROM context69.tasks)
  + (SELECT count(*) FROM context69.task_items)
  + (SELECT count(*) FROM context69.library_files)
  + (SELECT count(*) FROM context69.library_storage_objects)
  + (SELECT count(*) FROM context69.library_legacy_object_cleanup)
  + (SELECT count(*) FROM context69.library_storage_object_cleanup)
  + (SELECT count(*) FROM context69.metadata_index_definitions)
    AS "total!"
