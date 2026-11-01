SELECT id, group_id, source_key, external_id, record_hash
FROM context69.documents
WHERE id = ANY($1::bigint[])
ORDER BY id
FOR UPDATE
