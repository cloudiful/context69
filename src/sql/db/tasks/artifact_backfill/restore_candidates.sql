-- Metadata-only restore candidates for the inverse operator direction
-- (issue #975 P6).
--
-- One row per existing artifact reference joined with its registry object and
-- owning task/item. The projection carries the expected reference identity the
-- restore CAS re-checks, and the flags needed to classify exclusions. No field
-- text and no digest value is ever printed by the caller.
--
-- Rows are NOT filtered to eligible ones: an active item, a NULL group or an
-- already-restored inline field is reported as an explicit skip class instead
-- of silently disappearing from the counts.
SELECT artifact.item_id,
       item.task_id,
       artifact.kind,
       artifact.encoding,
       artifact.storage_object_id,
       object.group_id AS object_group_id,
       object.sha256,
       object.size_bytes,
       object.storage_backend,
       item.status AS item_status,
       task.status AS task_status,
       task.deleted_at IS NOT NULL AS task_deleted,
       (item.lease_token IS NOT NULL OR item.lease_until IS NOT NULL) AS lease_active,
       task.group_id AS task_group_id,
       CASE WHEN artifact.kind = 'download'
            THEN jsonb_typeof(item.payload #> '{download_artifact,content_base64}')
            ELSE jsonb_typeof(item.payload -> 'section_payload')
       END AS inline_type,
       CASE WHEN artifact.kind = 'download'
            THEN jsonb_typeof(item.payload -> 'download_artifact')
            ELSE 'object'
       END AS download_metadata_type,
       CASE WHEN artifact.kind = 'download'
            THEN item.payload #>> '{download_artifact,sha256}'
            ELSE NULL
       END AS declared_sha256
FROM context69.task_item_artifacts AS artifact
JOIN context69.library_storage_objects AS object
  ON object.id = artifact.storage_object_id
JOIN context69.task_items AS item
  ON item.id = artifact.item_id
JOIN context69.tasks AS task
  ON task.id = item.task_id
WHERE ($1::text = 'all' OR artifact.kind = $1)
  AND ($5::uuid[] IS NULL OR item.id = ANY($5::uuid[]))
  AND ($2::uuid IS NULL OR (item.id, artifact.kind) > ($2, $3))
ORDER BY item.id, artifact.kind
LIMIT $4
