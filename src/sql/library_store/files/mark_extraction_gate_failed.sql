-- Owned required-extraction gate failure mark (issue #89 P3).
--
-- Marks a `succeeded` file as `failed` only when the calling worker still owns
-- the task item. Ownership is the exact triple proven by `set_file.sql` and
-- `finish_item.sql`: (item.id, lease_token, status=running) plus the bound
-- file_id. The CTE materializes and locks the owned item row (`FOR UPDATE`)
-- so a concurrent cancel/re-claim cannot interleave between the ownership
-- check and the file update; an unlocked `EXISTS` check would allow a
-- stale worker to mark a new owner's file.
--
-- Only the current expected file status (normally `succeeded`) is updated: a
-- lost race, a cancelled item, or a file that already left the expected state
-- yields zero rows and the caller treats it as lost ownership, never success.
-- Converted/indexed artifacts are retained; this statement never deletes
-- documents, chunks, checkpoints, or payloads.
--
-- $1 file_id, $2 item_id, $3 error_message, $4 expected ingest_status,
-- $5 lease_token.
WITH owned AS MATERIALIZED (
    SELECT i.file_id AS file_id
    FROM context69.task_items AS i
    WHERE i.id = $2
      AND i.lease_token = $5
      AND i.status = 'running'
      AND i.file_id = $1
    FOR UPDATE
)
UPDATE context69.library_files AS f
SET ingest_status = 'failed',
    error_message = $3,
    updated_at = now()
FROM owned
WHERE f.id = owned.file_id
  AND f.id = $1
  AND f.ingest_status = $4
RETURNING
    f.group_id,
    (SELECT group_key FROM context69.groups WHERE id = f.group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = f.group_id) AS "group_path!",
    f.visibility,
    f.id,
    f.folder_id,
    f.external_id,
    f.source_uri,
    f.published_at,
    f.metadata_json,
    f.filename,
    f.media_type,
    f.size_bytes,
    f.sha256,
    f.storage_rel_path,
    f.ingest_status,
    f.error_message,
    f.created_at,
    f.updated_at,
    f.ingested_at
