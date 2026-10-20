-- Metadata-only terminal inline candidates for the pre-artifact preview
-- (issue #975 P5).
--
-- This query never references context69.task_item_artifacts: it must run on a
-- database where the artifact migration has not been applied yet. It returns
-- only lengths, JSON types and the presence of a declared digest; no field
-- text or digest value leaves the database.
WITH kinds(kind) AS (
    VALUES ('download'), ('sections')
), rows AS (
    SELECT item.id AS item_id,
           kinds.kind AS kind,
           item.task_id,
           task.group_id,
           item.status AS item_status,
           task.status AS task_status,
           task.deleted_at IS NOT NULL AS task_deleted,
           (item.lease_token IS NOT NULL OR item.lease_until IS NOT NULL) AS lease_active,
           CASE WHEN kinds.kind = 'download'
                THEN jsonb_typeof(item.payload #> '{download_artifact,content_base64}')
                ELSE jsonb_typeof(item.payload -> 'section_payload')
           END AS field_type,
           CASE WHEN kinds.kind = 'download'
                THEN octet_length(item.payload #>> '{download_artifact,content_base64}')::bigint
                ELSE octet_length(item.payload ->> 'section_payload')::bigint
           END AS encoded_len,
           CASE WHEN kinds.kind = 'download'
                THEN jsonb_typeof(item.payload #> '{download_artifact,sha256}') = 'string'
                ELSE false
           END AS declared_sha256_present,
           CASE WHEN kinds.kind = 'download'
                THEN (item.payload #>> '{download_artifact,sha256}') ~ '^[0-9a-f]{64}$'
                ELSE false
           END AS declared_sha256_valid
    FROM context69.task_items AS item
    JOIN context69.tasks AS task ON task.id = item.task_id
    CROSS JOIN kinds
    WHERE item.status IN ('succeeded', 'failed', 'cancelled')
      AND ($1::text = 'all' OR kinds.kind = $1)
      AND (
          (kinds.kind = 'download'
              AND item.payload #>> '{download_artifact,content_base64}' IS NOT NULL)
          OR (kinds.kind = 'sections'
              AND item.payload ->> 'section_payload' IS NOT NULL)
      )
      AND ($5::uuid[] IS NULL OR item.id = ANY($5::uuid[]))
      AND ($2::uuid IS NULL OR (item.id, kinds.kind) > ($2, $3))
)
SELECT item_id,
       kind,
       task_id,
       group_id,
       item_status,
       task_status,
       task_deleted,
       lease_active,
       field_type,
       encoded_len,
       declared_sha256_present,
       declared_sha256_valid,
       false AS artifact_present
FROM rows
ORDER BY item_id, kind
LIMIT $4
