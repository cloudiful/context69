-- Fetch one terminal item kind's encoded field once, gated in the same
-- snapshot (issue #975 P5, capped at the database boundary in P8).
--
-- Metadata classification happened in an earlier statement, so a concurrent
-- retry can rewrite the field between that scan and this fetch. The projection
-- therefore re-derives the owner state, the current field shape and the current
-- length here, and projects the text ONLY when the row is still an eligible
-- terminal owner and the text fits the caller's cap. Otherwise the text is
-- NULL and `skip_reason` names the refusal, so an oversized or changed field is
-- never materialized client-side. No substring windows are used; the row is
-- detoasted once by PostgreSQL as documented.
WITH current AS (
    SELECT item.id AS item_id,
           task.group_id,
           item.status AS item_status,
           task.status AS task_status,
           task.deleted_at AS deleted_at,
           (item.lease_token IS NOT NULL OR item.lease_until IS NOT NULL) AS lease_active,
           CASE WHEN $2::text = 'download'
                THEN item.payload #>> '{download_artifact,content_base64}'
                ELSE item.payload ->> 'section_payload'
           END AS field_text,
           CASE WHEN $2::text = 'download'
                THEN jsonb_typeof(item.payload #> '{download_artifact,content_base64}')
                ELSE jsonb_typeof(item.payload -> 'section_payload')
           END AS field_type,
           CASE WHEN $2::text = 'download'
                THEN item.payload #>> '{download_artifact,sha256}'
                ELSE NULL
           END AS declared_sha256
    FROM context69.task_items AS item
    JOIN context69.tasks AS task ON task.id = item.task_id
    WHERE item.id = $1
), classified AS (
    SELECT current.*,
           CASE
               WHEN current.field_type IS NULL THEN 'no_inline'
               WHEN current.item_status NOT IN ('succeeded', 'failed', 'cancelled')
                   THEN 'race_changed'
               WHEN current.task_status NOT IN ('succeeded', 'failed', 'cancelled')
                   THEN 'race_changed'
               WHEN current.deleted_at IS NOT NULL THEN 'race_changed'
               WHEN current.lease_active THEN 'race_changed'
               WHEN current.group_id IS NULL THEN 'race_changed'
               WHEN current.field_type <> CASE WHEN $2::text = 'download'
                                               THEN 'string'
                                               ELSE 'array'
                                          END
                   THEN 'invalid_shape'
               WHEN octet_length(current.field_text) > $3::bigint THEN 'oversize'
               ELSE NULL
           END AS skip_reason
    FROM current
)
SELECT item_id,
       group_id,
       item_status,
       task_status,
       deleted_at IS NOT NULL AS task_deleted,
       lease_active,
       field_type,
       skip_reason,
       CASE WHEN skip_reason IS NULL THEN field_text ELSE NULL END AS field_text,
       declared_sha256
FROM classified
