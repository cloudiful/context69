-- Hot-path indexes for the post-#529 schema (issue #530 Task 1).
--
-- Two remaining hot paths get an index:
--   * idx_documents_library_file_id serves the `metadata_json->>'library_file_id'`
--     equality in delete_documents_for_library_file.sql and
--     list_chunk_ids_for_library_file.sql, which previously seq-scanned
--     context69.documents and cascaded into the chunk tables.
--   * idx_task_items_status_next_attempt serves the claim selection in
--     claim_items.sql, whose eligible predicate filters item status and due
--     next_attempt_at. No existing index covers that column pair:
--     idx_task_items_stage_status leads with the collapsed `stage` column and
--     idx_task_items_lease covers (status, lease_until). The partial predicate
--     mirrors the three statuses the claim path can touch.
--
-- Plain (transactional) CREATE INDEX on purpose: the sqlx migrator sends a
-- migration file as one simple-query message, so all statements of a file share
-- a single implicit transaction and CREATE INDEX CONCURRENTLY is rejected there
-- once a file holds more than one statement. A one-statement CONCURRENTLY
-- migration does run, but both indexes belong to this file. To avoid the write
-- lock on the large context69.task_items table, the operator may pre-create
-- either index CONCURRENTLY out of band before applying this migration
-- (issue #530 Task 4); IF NOT EXISTS then turns the statement into a no-op.
--
-- Deliberately not created (already covered by the simplified schema):
--   * documents(group_id, source_key) is a prefix of
--     idx_documents_group_source_published_id (group_id, source_key,
--     published_at DESC, id).
--   * task_external_jobs(provider, last_polled_at) is obsolete: the table was
--     dropped by migration 20260920180515_drop_docling_async.sql.
--
-- Rollback: DROP INDEX (CONCURRENTLY) each index; no data is touched.

CREATE INDEX IF NOT EXISTS idx_documents_library_file_id
    ON context69.documents (((metadata_json ->> 'library_file_id')));

CREATE INDEX IF NOT EXISTS idx_task_items_status_next_attempt
    ON context69.task_items (status, next_attempt_at)
    WHERE status IN ('queued', 'waiting', 'running');
