WITH target_group AS (
    SELECT id FROM context69.groups WHERE group_key = $1
), deleted_documents AS (
    DELETE FROM context69.documents
    WHERE group_id IN (SELECT id FROM target_group)
    RETURNING id
), deleted_group AS (
    DELETE FROM context69.groups
    WHERE id IN (SELECT id FROM target_group)
      AND (SELECT count(*) FROM deleted_documents) >= 0
    RETURNING id
)
SELECT id FROM deleted_group
